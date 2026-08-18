use opencv::{
    core::{self, BORDER_DEFAULT, Mat, Point, Point2f, Size, Vector},
    geometry,
    imgproc,
    prelude::*,
};
use serde::{Deserialize, Serialize};

use super::DigitizationError;

// Локальный аналог точки для безопасного маршалинга в JSON
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomPoint {
    pub x: i32,
    pub y: i32,
}

// Финальная структура вершин, которая летит через Axum сетевой шлюз во Flutter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageVertices {
    pub p1: CustomPoint,
    pub p2: CustomPoint,
    pub p3: CustomPoint,
    pub p4: CustomPoint,
}

/// B3: Оценка качества кандидата маски страницы.
/// Чем выше score — тем лучше кандидат:
///   - площадь в диапазоне 10%–95% кадра (страница, а не лампа/тень);
///   - высокая выпуклость (страница — почти выпуклый объект);
///   - компактность (площадь / площадь выпуклой оболочки).
fn mask_candidate_score(contour: &Vector::<Point>, image_area: f64) -> f64 {
    let area = geometry::contour_area(contour, false).unwrap_or(0.0);
    if area < image_area * 0.10 || area > image_area * 0.95 {
        return 0.0;
    }

    let mut hull = Vector::<Point>::new();
    if geometry::convex_hull(contour, &mut hull, false, true).is_err() {
        return 0.0;
    }
    let hull_area = geometry::contour_area(&hull, false).unwrap_or(0.0);
    if hull_area <= 0.0 {
        return 0.0;
    }

    let solidity = area / hull_area; // 0..1, страница близка к 1.0
    solidity
}

/// Coarse masking: отсечь потолок/лампы при открытой крышке сканера.
/// B3: мультимасштабный алгоритм для сложных сценариев освещения:
///   1. Grayscale.
///   2. Несколько масштабов размытия (51/101/201) → Otsu INV на каждом.
///   3. На каждом масштабе — кандидаты контуров, оценка solidity.
///   4. Лучший кандидат (макс. solidity при валидной площади).
///   5. Морфологическое закрытие маски (заполнение провалов у ламп).
///   6. bitwise_and к исходному изображению.
/// Возвращает очищенный кадр без ярких артефактов за пределами страницы.
fn coarse_mask(src: &Mat) -> Result<Mat, DigitizationError> {
    let mut gray = Mat::default();
    if src.channels() > 1 {
        imgproc::cvt_color(
            src,
            &mut gray,
            imgproc::COLOR_BGR2GRAY,
            0,
            core::AlgorithmHint::ALGO_HINT_APPROX,
        )
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    } else {
        gray = src.clone();
    }

    let image_area = src.rows() as f64 * src.cols() as f64;

    // Мультимасштабный поиск лучшего кандидата
    let blur_sizes = [51_i32, 101_i32, 201_i32];
    let mut best_score = 0.0_f64;
    let mut best_contour: Option<Vector::<Point>> = None;

    for &ksize in &blur_sizes {
        let mut blurred = Mat::default();
        imgproc::gaussian_blur(
            &gray,
            &mut blurred,
            Size::new(ksize, ksize),
            0.0,
            0.0,
            BORDER_DEFAULT,
            core::AlgorithmHint::ALGO_HINT_APPROX,
        )
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

        let mut thresh = Mat::default();
        imgproc::threshold(&blurred, &mut thresh, 0.0, 255.0, imgproc::THRESH_BINARY_INV + imgproc::THRESH_OTSU)
            .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

        let mut contours_vec = Vector::<Vector::<Point>>::new();
        imgproc::find_contours(
            &thresh,
            &mut contours_vec,
            imgproc::RETR_EXTERNAL,
            imgproc::CHAIN_APPROX_SIMPLE,
            Point::new(0, 0),
        )
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

        for i in 0..contours_vec.len() {
            let contour = contours_vec.get(i).map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
            let score = mask_candidate_score(&contour, image_area);
            if score > best_score {
                best_score = score;
                best_contour = Some(contour.clone());
            }
        }
    }

    let best_contour = match best_contour {
        Some(c) => c,
        None => return Ok(src.clone()), // страница на весь кадр — маска не нужна
    };

    // Строим маску из лучшего кандидата
    let mut mask = Mat::zeros(src.rows(), src.cols(), core::CV_8UC1)
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?
        .to_mat()
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    let mut single_contour = Vector::<Vector::<Point>>::new();
    single_contour.push(best_contour.clone());
    let empty_hierarchy = Mat::default();
    imgproc::draw_contours(
        &mut mask,
        &single_contour,
        0,
        core::Scalar::all(255.0),
        -1,
        -1,
        &empty_hierarchy,
        0,
        Point::new(0, 0),
    )
    .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    // Морфологическое закрытие: заполняем провалы (тени ламп, складки)
    let close_kernel = imgproc::get_structuring_element(
        imgproc::MORPH_ELLIPSE,
        Size::new(15, 15),
        Point::new(0, 0),
    )
    .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    let mut closed_mask = Mat::default();
    imgproc::morphology_ex(
        &mask,
        &mut closed_mask,
        imgproc::MORPH_CLOSE,
        &close_kernel,
        Point::new(-1, -1),
        2,
        BORDER_DEFAULT,
        core::Scalar::all(0.0),
    )
    .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    let mut result = Mat::default();
    core::bitwise_and(src, &closed_mask, &mut result, &Mat::default())
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    Ok(result)
}

/// B2: Изоляция боковых артефактов ("боковушек").
///
/// Градиентный анализ плотности по периферии macro-contour:
///   1. Строит маску крупнейшего контура.
///   2. Извлекает полосу (band) шириной `band_width` пикселей вокруг контура.
///   3. Вычисляет частоту чередования светлых/тёмных пикселей вдоль периметра.
///   4. Если частота выше порога — паттерн "боковушек" обнаружен.
///   5. Эродирует маску на `erode_iters` итераций для сдвига рамки внутрь.
///
/// Возвращает улучшенную маску (CV_8UC1, 0/255).
fn isolate_side_artifacts(
    src: &Mat,
    mask: &Mat,
    band_width: i32,
    erode_iters: i32,
) -> Result<Mat, DigitizationError> {
    let rows = mask.rows() as usize;
    let cols = mask.cols() as usize;

    // 1. Извлекаем полосу вокруг контура: XOR между маской и её эродированной версией
    let kernel = imgproc::get_structuring_element(
        imgproc::MORPH_RECT,
        Size::new(band_width, band_width),
        Point::new(0, 0),
    )
    .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    let mut eroded = Mat::default();
    imgproc::erode(
        mask,
        &mut eroded,
        &kernel,
        Point::new(-1, -1),
        1,
        BORDER_DEFAULT,
        core::Scalar::all(0.0),
    )
    .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    // Полоса = маска XOR эродированная маска
    let mut band = Mat::default();
    core::bitwise_xor(mask, &eroded, &mut band, &Mat::default())
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    // 2. Анализируем полосу: считаем чередования по строкам
    let mut gray_band = Mat::default();
    if src.channels() > 1 {
        imgproc::cvt_color(
            src,
            &mut gray_band,
            imgproc::COLOR_BGR2GRAY,
            0,
            core::AlgorithmHint::ALGO_HINT_APPROX,
        )
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    } else {
        gray_band = src.clone();
    }

    // Считаем частоту чередований в полосе
    let mut transitions: i32 = 0;
    let mut total_samples: i32 = 0;

    unsafe {
        let band_data = std::slice::from_raw_parts(band.data() as *const u8, rows * cols);
        let gray_data = std::slice::from_raw_parts(gray_band.data() as *const u8, rows * cols);

        for y in 0..rows {
            let mut prev_state: Option<bool> = None;
            for x in 0..cols {
                let idx = y * cols + x;
                if band_data[idx] > 0 {
                    // Пиксель в полосе — определяем светлый/тёмный
                    let is_dark = gray_data[idx] < 128;
                    if let Some(prev) = prev_state {
                        if prev != is_dark {
                            transitions += 1;
                        }
                    }
                    prev_state = Some(is_dark);
                    total_samples += 1;
                }
            }
        }
    }

    // 3. Если частота чередований > 30% — это "боковушки"
    let transition_ratio = if total_samples > 10 {
        transitions as f64 / total_samples as f64
    } else {
        0.0
    };

    if transition_ratio > 0.30 && erode_iters > 0 {
        // Эродируем маску для сдвига внутрь
        let erode_kernel = imgproc::get_structuring_element(
            imgproc::MORPH_RECT,
            Size::new(3, 3),
            Point::new(0, 0),
        )
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

        let mut result = mask.clone();
        for _ in 0..erode_iters {
            let mut eroded_iter = Mat::default();
            imgproc::erode(
                &result,
                &mut eroded_iter,
                &erode_kernel,
                Point::new(-1, -1),
                1,
                BORDER_DEFAULT,
                core::Scalar::all(0.0),
            )
            .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
            result = eroded_iter;
        }
        return Ok(result);
    }

    Ok(mask.clone())
}

/// Детекция вершин страницы на бинарном изображении.
/// Алгоритм:
///   1. Coarse masking для отсечения артефактов.
///   2. Бинаризация Otsu.
///   3. Поиск контуров → крупнейший контур.
///   4. B2: Изоляция боковых артефактов (эрозия при обнаружении паттерна).
///   5. Аппроксимация полигоном (approxPolyDP) до 4 точек.
///   6. Если не 4 точки → minAreaRect.
///   7. Сортировка вершин TL → TR → BR → BL.
pub fn process_book_contours(src: &Mat) -> Result<PageVertices, DigitizationError> {
    // Coarse masking
    let masked = coarse_mask(src)?;

    // Грейскейл
    let mut gray = Mat::default();
    if masked.channels() > 1 {
        imgproc::cvt_color(
            &masked,
            &mut gray,
            imgproc::COLOR_BGR2GRAY,
            0,
            core::AlgorithmHint::ALGO_HINT_APPROX,
        )
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    } else {
        gray = masked.clone();
    }

    // Бинаризация Otsu
    let mut thresh = Mat::default();
    imgproc::threshold(&gray, &mut thresh, 0.0, 255.0, imgproc::THRESH_BINARY_INV + imgproc::THRESH_OTSU)
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    // Поиск контуров
    let mut contours_vec = Vector::<Vector::<Point>>::new();
    imgproc::find_contours(
        &thresh,
        &mut contours_vec,
        imgproc::RETR_EXTERNAL,
        imgproc::CHAIN_APPROX_SIMPLE,
        Point::new(0, 0),
    )
    .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    if contours_vec.is_empty() {
        return Err(DigitizationError::NoContourFound(
            "Не обнаружено ни одного контура для детекции страницы".to_string(),
        ));
    }

    // Самый большой контур по площади
    let mut max_area = 0.0_f64;
    let mut best_idx = 0_usize;
    for i in 0..contours_vec.len() {
        let contour = contours_vec.get(i).map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
        let area = geometry::contour_area(&contour, false).unwrap_or(0.0);
        if area > max_area {
            max_area = area;
            best_idx = i;
        }
    }

    if max_area < (src.rows() as f64 * src.cols() as f64 * 0.01) {
        return Err(DigitizationError::DegenerateContour(format!(
            "Самый крупный контур слишком мал: {} px²",
            max_area
        )));
    }

    let best_contour = contours_vec.get(best_idx).map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    // B2: Изоляция боковых артефактов — эрозия маски при обнаружении паттерна
    let mut page_mask = Mat::zeros(src.rows(), src.cols(), core::CV_8UC1)
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?
        .to_mat()
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    let empty_hier = Mat::default();
    imgproc::draw_contours(
        &mut page_mask,
        &contours_vec,
        best_idx as i32,
        core::Scalar::all(255.0),
        -1,
        -1,
        &empty_hier,
        0,
        Point::new(0, 0),
    )
    .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    let refined_mask = isolate_side_artifacts(src, &page_mask, 8, 3)?;

    // Пересчитываем контур из улучшенной маски
    let mut refined_thresh = Mat::default();
    imgproc::threshold(&refined_mask, &mut refined_thresh, 127.0, 255.0, imgproc::THRESH_BINARY)
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    let mut refined_contours = Vector::<Vector::<Point>>::new();
    imgproc::find_contours(
        &refined_thresh,
        &mut refined_contours,
        imgproc::RETR_EXTERNAL,
        imgproc::CHAIN_APPROX_SIMPLE,
        Point::new(0, 0),
    )
    .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    let best_contour = match refined_contours.get(0) {
        Ok(rc) => rc.clone(),
        Err(_) => best_contour.clone(),
    };

    // Аппроксимация четырёхугольником
    let perimeter = geometry::arc_length(&best_contour, true).unwrap_or(0.0);
    if perimeter < 1.0 {
        return Err(DigitizationError::DegenerateContour(
            "Периметр контура слишком мал для аппроксимации".to_string(),
        ));
    }

    let epsilon = perimeter * 0.02;
    let mut approx_vec = Vector::<Point>::new();
    geometry::approx_poly_dp(&best_contour, &mut approx_vec, epsilon, true)
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    let pts: Vec<Point> = approx_vec.to_vec();

    if pts.len() == 4 {
        return Ok(sort_four_points(pts));
    }

    // minAreaRect как последний шанс
    let rect = geometry::min_area_rect(&best_contour).map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    let mut pts_f: [Point2f; 4] = unsafe { std::mem::zeroed() };
    rect.points(&mut pts_f).map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    let corners: Vec<Point> = pts_f.iter()
        .map(|p| Point::new(p.x as i32, p.y as i32))
        .collect();

    if corners.len() == 4 {
        return Ok(sort_four_points(corners));
    }

    Err(DigitizationError::InvalidPageGeometry(
        "Не удалось получить четыре вершины страницы ни одним методом".to_string(),
    ))
}

/// Детекция угла скоса страницы по проекционной линии.
/// Алгоритм:
///   1. Бинаризация Otsu.
///   2. Горизонтальная проекция (сумма черных пикселей по строкам).
///   3. Поиск пиковов проекции (строки с текстом).
///   4. Линейная регрессия по пикам → угол наклона.
pub fn detect_skew_angle(src: &Mat) -> Result<f64, DigitizationError> {
    // Грейскейл
    let mut gray = Mat::default();
    if src.channels() > 1 {
        imgproc::cvt_color(
            src,
            &mut gray,
            imgproc::COLOR_BGR2GRAY,
            0,
            core::AlgorithmHint::ALGO_HINT_APPROX,
        )
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    } else {
        gray = src.clone();
    }

    // Бинаризация Otsu
    let mut thresh = Mat::default();
    imgproc::threshold(&gray, &mut thresh, 0.0, 255.0, imgproc::THRESH_BINARY_INV + imgproc::THRESH_OTSU)
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    // Горизонтальная проекция
    let rows = thresh.rows() as usize;
    let cols = thresh.cols() as usize;
    let mut projection = vec![0.0_f64; rows];

    for row in 0..rows {
        let row_data = thresh.row(row as i32).map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
        let row_vec: Vec<Vec<u8>> = row_data.to_vec_2d().map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
        for col in 0..cols {
            if row_vec[0][col] > 128 {
                projection[row] += 1.0;
            }
        }
    }

    // Поиск пиковов (строки с текстом)
    let threshold = projection.iter().sum::<f64>() / (projection.len() as f64) * 0.5;
    let mut peaks: Vec<(i32, f64)> = Vec::new();
    for (row, &val) in projection.iter().enumerate() {
        if val > threshold {
            peaks.push((row as i32, val));
        }
    }

    if peaks.len() < 2 {
        return Ok(0.0);
    }

    // Линейная регрессия: y = a*x + b, где x = row, y = projection[row]
    let n = peaks.len() as f64;
    let sum_x = peaks.iter().map(|(x, _)| *x as f64).sum::<f64>();
    let sum_y = peaks.iter().map(|(_, y)| *y).sum::<f64>();
    let sum_xy = peaks.iter().map(|(x, y)| (*x as f64) * *y).sum::<f64>();
    let sum_x2 = peaks.iter().map(|(x, _)| (*x as f64) * (*x as f64)).sum::<f64>();

    let denom = n * sum_x2 - sum_x * sum_x;
    if denom.abs() < 1e-6 {
        return Ok(0.0);
    }

    let a = (n * sum_xy - sum_x * sum_y) / denom;
    let angle = (a as f64).atan() * 180.0 / std::f64::consts::PI;

    Ok(angle)
}

/// Поворот изображения на заданный угол.
pub fn rotate_image(src: &Mat, angle: f64) -> Result<Mat, DigitizationError> {
    if angle.abs() < 0.1 {
        return Ok(src.clone());
    }

    let size = src.size().map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    let center = Point2f::new(size.width as f32 / 2.0, size.height as f32 / 2.0);

    let matrix = geometry::get_rotation_matrix_2d(center, angle, 1.0)
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    let mut dst = Mat::default();
    imgproc::warp_affine(
        src,
        &mut dst,
        &matrix,
        size,
        imgproc::INTER_LINEAR,
        core::BORDER_REPLICATE,
        core::Scalar::all(255.0),
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )
    .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    Ok(dst)
}

/// Сегментация разворота на левую и правую страницы.
/// Алгоритм:
///   1. Найти середину разворота по X.
///   2. Разделить изображение на две половины.
///   3. Для каждой половины найти bounding box контента.
///   4. Вернуть обрезанные страницы.
pub fn segment_pages(src: &Mat) -> Result<(Mat, Mat), DigitizationError> {
    let size = src.size().map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    let half_width = size.width / 2;

    // Разделить на левую и правую половины
    let left_roi = Mat::col_bounds(src, 0, half_width).map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    let right_roi = Mat::col_bounds(src, half_width, size.width).map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    let left_mat = left_roi.clone_pointee();
    let right_mat = right_roi.clone_pointee();

    // Обрезать каждую половину до контента
    let left_cropped = crop_to_content(&left_mat)?;
    let right_cropped = crop_to_content(&right_mat)?;

    Ok((left_cropped, right_cropped))
}

/// Обрезка изображения до bounding box контента.
fn crop_to_content(src: &Mat) -> Result<Mat, DigitizationError> {
    // Грейскейл
    let mut gray = Mat::default();
    if src.channels() > 1 {
        imgproc::cvt_color(
            src,
            &mut gray,
            imgproc::COLOR_BGR2GRAY,
            0,
            core::AlgorithmHint::ALGO_HINT_APPROX,
        )
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    } else {
        gray = src.clone();
    }

    // Бинаризация Otsu
    let mut thresh = Mat::default();
    imgproc::threshold(&gray, &mut thresh, 0.0, 255.0, imgproc::THRESH_BINARY_INV + imgproc::THRESH_OTSU)
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    // Найти bounding box
    let bbox = geometry::bounding_rect(&thresh).map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    // Проверка валидности
    if bbox.width <= 0 || bbox.height <= 0 {
        return Ok(src.clone());
    }

    // Обрезать
    let roi = src.roi(bbox).map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    Ok(roi.clone_pointee())
}

/// Сортировка четырёх точек в порядке TL → TR → BR → BL
fn sort_four_points(mut pts: Vec<Point>) -> PageVertices {
    // Сортируем по сумме x+y (диагональная проекция)
    pts.sort_by_key(|p| p.x + p.y);

    let top_pair = [&pts[0], &pts[1]];
    let bottom_pair = [&pts[2], &pts[3]];

    // В верхней паре левая — с меньшим X
    let (tl, tr) = if top_pair[0].x <= top_pair[1].x {
        (top_pair[0], top_pair[1])
    } else {
        (top_pair[1], top_pair[0])
    };

    // В нижней паре левая — с меньшим X
    let (bl, br) = if bottom_pair[0].x <= bottom_pair[1].x {
        (bottom_pair[0], bottom_pair[1])
    } else {
        (bottom_pair[1], bottom_pair[0])
    };

    PageVertices {
        p1: CustomPoint { x: tl.x, y: tl.y },
        p2: CustomPoint { x: tr.x, y: tr.y },
        p3: CustomPoint { x: br.x, y: br.y },
        p4: CustomPoint { x: bl.x, y: bl.y },
    }
}