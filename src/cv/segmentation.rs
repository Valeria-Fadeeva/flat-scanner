use opencv::{
    core::{self, BORDER_DEFAULT, Mat, Point, Point2f, Size, Vector},
    geometry,
    imgproc,
    prelude::*,
};
use serde::{Deserialize, Serialize};

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

/// Coarse masking: отсечь потолок/лампы при открытой крышке сканера.
/// Алгоритм:
///   1. Сильное размытие → Otsu бинаризация (страница обычно темнее фона).
///   2. Найти крупнейший контур по площади.
///   3. Создать одноканальную маску из этого контура.
///   4. Применить bitwise_and к исходному изображению.
/// Возвращает очищенный кадр без ярких артефактов за пределами страницы.
fn coarse_mask(src: &Mat) -> Result<Mat, String> {
    let mut gray = Mat::default();
    imgproc::cvt_color(
        src,
        &mut gray,
        imgproc::COLOR_BGR2GRAY,
        0,
        core::AlgorithmHint::ALGO_HINT_APPROX,
    )
    .map_err(|e| e.to_string())?;

    let mut blurred = Mat::default();
    imgproc::gaussian_blur(
        &gray,
        &mut blurred,
        Size::new(51, 51),
        0.0,
        0.0,
        BORDER_DEFAULT,
        core::AlgorithmHint::ALGO_HINT_APPROX,
    )
    .map_err(|e| e.to_string())?;

    let mut thresh = Mat::default();
    imgproc::threshold(&blurred, &mut thresh, 0.0, 255.0, imgproc::THRESH_BINARY_INV + imgproc::THRESH_OTSU)
        .map_err(|e| e.to_string())?;

    let mut contours_vec = Vector::<Vector::<Point>>::new();
    imgproc::find_contours(
        &thresh,
        &mut contours_vec,
        imgproc::RETR_EXTERNAL,
        imgproc::CHAIN_APPROX_SIMPLE,
        Point::new(0, 0),
    )
    .map_err(|e| e.to_string())?;

    if contours_vec.is_empty() {
        return Ok(src.clone());
    }

    let mut max_area = 0.0_f64;
    let mut best_idx = 0_usize;
    for i in 0..contours_vec.len() {
        let contour = contours_vec.get(i).map_err(|e| e.to_string())?;
        let area = geometry::contour_area(&contour, false).unwrap_or(0.0);
        if area > max_area {
            max_area = area;
            best_idx = i;
        }
    }

    // Если крупнейший объект слишком мал — маска не нужна (возможно, страница на весь кадр)
    let image_area = src.rows() as f64 * src.cols() as f64;
    if max_area < image_area * 0.05 {
        return Ok(src.clone());
    }

    let mut mask = Mat::zeros(src.rows(), src.cols(), core::CV_8UC1)
        .map_err(|e| e.to_string())?
        .to_mat()
        .map_err(|e| e.to_string())?;

    // Пустая иерархия для draw_contours
    let empty_hierarchy = Mat::default();
    imgproc::draw_contours(
        &mut mask,
        &contours_vec,
        best_idx as i32,
        core::Scalar::all(255.0),
        -1,
        -1,
        &empty_hierarchy,
        0,
        Point::new(0, 0),
    )
    .map_err(|e| e.to_string())?;

    let mut result = Mat::default();
    core::bitwise_and(src, &mask, &mut result, &Mat::default())
        .map_err(|e| e.to_string())?;

    Ok(result)
}

/// Детекция вершин страницы на бинарном изображении.
/// Алгоритм:
///   1. Coarse masking для отсечения артефактов.
///   2. Бинаризация Otsu.
///   3. Поиск контуров → крупнейший контур.
///   4. Аппроксимация полигоном (approxPolyDP) до 4 точек.
///   5. Если не 4 точки → minAreaRect.
///   6. Сортировка вершин TL → TR → BR → BL.
pub fn process_book_contours(src: &Mat) -> Result<PageVertices, String> {
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
        .map_err(|e| e.to_string())?;
    } else {
        gray = masked.clone();
    }

    // Бинаризация Otsu
    let mut thresh = Mat::default();
    imgproc::threshold(&gray, &mut thresh, 0.0, 255.0, imgproc::THRESH_BINARY_INV + imgproc::THRESH_OTSU)
        .map_err(|e| e.to_string())?;

    // Поиск контуров
    let mut contours_vec = Vector::<Vector::<Point>>::new();
    imgproc::find_contours(
        &thresh,
        &mut contours_vec,
        imgproc::RETR_EXTERNAL,
        imgproc::CHAIN_APPROX_SIMPLE,
        Point::new(0, 0),
    )
    .map_err(|e| e.to_string())?;

    if contours_vec.is_empty() {
        return Err("Не обнаружено ни одного контура для детекции страницы".to_string());
    }

    // Самый большой контур по площади
    let mut max_area = 0.0_f64;
    let mut best_idx = 0_usize;
    for i in 0..contours_vec.len() {
        let contour = contours_vec.get(i).map_err(|e| e.to_string())?;
        let area = geometry::contour_area(&contour, false).unwrap_or(0.0);
        if area > max_area {
            max_area = area;
            best_idx = i;
        }
    }

    if max_area < (src.rows() as f64 * src.cols() as f64 * 0.01) {
        return Err(format!("Самый крупный контур слишком мал: {} px²", max_area));
    }

    let best_contour = contours_vec.get(best_idx).map_err(|e| e.to_string())?;

    // Аппроксимация четырёхугольником
    let perimeter = geometry::arc_length(&best_contour, true).unwrap_or(0.0);
    if perimeter < 1.0 {
        return Err("Периметр контура слишком мал для аппроксимации".to_string());
    }

    let epsilon = perimeter * 0.02;
    let mut approx_vec = Vector::<Point>::new();
    geometry::approx_poly_dp(&best_contour, &mut approx_vec, epsilon, true)
        .map_err(|e| e.to_string())?;

    let pts: Vec<Point> = approx_vec.to_vec();

    if pts.len() == 4 {
        return Ok(sort_four_points(pts));
    }

    // minAreaRect как последний шанс
    let rect = geometry::min_area_rect(&best_contour).map_err(|e| e.to_string())?;
    let mut pts_f: [Point2f; 4] = unsafe { std::mem::zeroed() };
    rect.points(&mut pts_f).map_err(|e| e.to_string())?;
    let corners: Vec<Point> = pts_f.iter()
        .map(|p| Point::new(p.x as i32, p.y as i32))
        .collect();

    if corners.len() == 4 {
        return Ok(sort_four_points(corners));
    }

    Err("Не удалось получить четыре вершины страницы ни одним методом".to_string())
}

/// Детекция угла скоса страницы по проекционной линии.
/// Алгоритм:
///   1. Бинаризация Otsu.
///   2. Горизонтальная проекция (сумма черных пикселей по строкам).
///   3. Поиск пиковов проекции (строки с текстом).
///   4. Линейная регрессия по пикам → угол наклона.
pub fn detect_skew_angle(src: &Mat) -> Result<f64, String> {
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
        .map_err(|e| e.to_string())?;
    } else {
        gray = src.clone();
    }

    // Бинаризация Otsu
    let mut thresh = Mat::default();
    imgproc::threshold(&gray, &mut thresh, 0.0, 255.0, imgproc::THRESH_BINARY_INV + imgproc::THRESH_OTSU)
        .map_err(|e| e.to_string())?;

    // Горизонтальная проекция
    let rows = thresh.rows() as usize;
    let cols = thresh.cols() as usize;
    let mut projection = vec![0.0_f64; rows];

    for row in 0..rows {
        let row_data = thresh.row(row as i32).map_err(|e| e.to_string())?;
        let row_vec: Vec<Vec<u8>> = row_data.to_vec_2d().map_err(|e| e.to_string())?;
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
pub fn rotate_image(src: &Mat, angle: f64) -> Result<Mat, String> {
    if angle.abs() < 0.1 {
        return Ok(src.clone());
    }

    let size = src.size().map_err(|e| e.to_string())?;
    let center = Point2f::new(size.width as f32 / 2.0, size.height as f32 / 2.0);

    let matrix = geometry::get_rotation_matrix_2d(center, angle, 1.0)
        .map_err(|e| e.to_string())?;

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
    .map_err(|e| e.to_string())?;

    Ok(dst)
}

/// Сегментация разворота на левую и правую страницы.
/// Алгоритм:
///   1. Найти середину разворота по X.
///   2. Разделить изображение на две половины.
///   3. Для каждой половины найти bounding box контента.
///   4. Вернуть обрезанные страницы.
pub fn segment_pages(src: &Mat) -> Result<(Mat, Mat), String> {
    let size = src.size().map_err(|e| e.to_string())?;
    let half_width = size.width / 2;

    // Разделить на левую и правую половины
    let left_roi = Mat::col_bounds(src, 0, half_width).map_err(|e| e.to_string())?;
    let right_roi = Mat::col_bounds(src, half_width, size.width).map_err(|e| e.to_string())?;

    let left_mat = left_roi.clone_pointee();
    let right_mat = right_roi.clone_pointee();

    // Обрезать каждую половину до контента
    let left_cropped = crop_to_content(&left_mat)?;
    let right_cropped = crop_to_content(&right_mat)?;

    Ok((left_cropped, right_cropped))
}

/// Обрезка изображения до bounding box контента.
fn crop_to_content(src: &Mat) -> Result<Mat, String> {
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
        .map_err(|e| e.to_string())?;
    } else {
        gray = src.clone();
    }

    // Бинаризация Otsu
    let mut thresh = Mat::default();
    imgproc::threshold(&gray, &mut thresh, 0.0, 255.0, imgproc::THRESH_BINARY_INV + imgproc::THRESH_OTSU)
        .map_err(|e| e.to_string())?;

    // Найти bounding box
    let bbox = geometry::bounding_rect(&thresh).map_err(|e| e.to_string())?;

    // Проверка валидности
    if bbox.width <= 0 || bbox.height <= 0 {
        return Ok(src.clone());
    }

    // Обрезать
    let roi = src.roi(bbox).map_err(|e| e.to_string())?;
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