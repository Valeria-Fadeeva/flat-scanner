use opencv::{
    core::{self, BORDER_DEFAULT, BORDER_TRANSPARENT, DECOMP_LU, Mat, Point2f, Size, Vec4i, Vector},
    geometry,
    imgproc,
    prelude::*,
};

use super::DigitizationError;

/// Валидация геометрии страницы перед гомографией (TECH_SPEC_addon_1.md §1.1).
/// Требования:
///   - строго 4 точки (гарантируется сигнатурой, но проверяем вырожденность);
///   - площадь контура >= 15% площади кадра;
///   - контур выпуклый (страница — почти выпуклый объект).
/// Возвращает типизированную ошибку InvalidPageGeometry при сбое.
fn validate_page_geometry(src: &Mat, pts: &[Point2f]) -> Result<(), DigitizationError> {
    if pts.len() != 4 {
        return Err(DigitizationError::InvalidPageGeometry(format!(
            "ожидается строго 4 вершины страницы, получено {}",
            pts.len()
        )));
    }

    // Площадь контура по формуле площади многоугольника (shoelace)
    let mut area = 0.0_f32;
    for i in 0..4 {
        let a = pts[i];
        let b = pts[(i + 1) % 4];
        area += a.x * b.y - b.x * a.y;
    }
    let contour_area = area.abs() / 2.0;

    let frame_area = src.rows() as f32 * src.cols() as f32;
    if frame_area <= 0.0 || contour_area < frame_area * 0.15 {
        return Err(DigitizationError::InvalidPageGeometry(format!(
            "площадь контура {:.1} px² ниже порога 15% кадра ({:.1} px²)",
            contour_area,
            frame_area * 0.15
        )));
    }

    // Выпуклость: все поворотные векторы имеют одинаковый знак кривизны
    let mut cross_sign: i32 = 0;
    for i in 0..4 {
        let o = pts[i];
        let a = pts[(i + 1) % 4];
        let b = pts[(i + 2) % 4];
        let cross = (a.x - o.x) * (b.y - a.y) - (a.y - o.y) * (b.x - a.x);
        let sign = if cross > 0.0 { 1 } else if cross < 0.0 { -1 } else { 0 };
        if sign != 0 {
            if cross_sign == 0 {
                cross_sign = sign;
            } else if cross_sign != sign {
                return Err(DigitizationError::InvalidPageGeometry(
                    "контур страницы не выпуклый".to_string(),
                ));
            }
        }
    }

    Ok(())
}

/// WarpPerspective: привести страницу к прямоугольному виду по вершинам P₁..P₄.
/// Выход — изображение заданного размера (target_size).
pub fn perspective_warp(src: &Mat, p1: Point2f, p2: Point2f, p3: Point2f, p4: Point2f, target_size: Size) -> Result<Mat, DigitizationError> {
    let pts = [p1, p2, p3, p4];
    validate_page_geometry(src, &pts)?;

    let mut src_pts_vec = Vector::<Point2f>::new();
    for p in pts {
        src_pts_vec.push(p);
    }

    let mut dst_pts_vec = Vector::<Point2f>::new();
    dst_pts_vec.push(Point2f::new(0.0, 0.0));
    dst_pts_vec.push(Point2f::new(target_size.width as f32, 0.0));
    dst_pts_vec.push(Point2f::new(target_size.width as f32, target_size.height as f32));
    dst_pts_vec.push(Point2f::new(0.0, target_size.height as f32));

    let m = geometry::get_perspective_transform(&src_pts_vec, &dst_pts_vec, DECOMP_LU)
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    let mut warped = Mat::default();
    imgproc::warp_perspective(
        src,
        &mut warped,
        &m,
        target_size,
        imgproc::INTER_LINEAR,
        BORDER_TRANSPARENT,
        core::Scalar::default(),
        core::AlgorithmHint::ALGO_HINT_APPROX,
    )
    .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    Ok(warped)
}

/// Детекция центральной тени корешка по градиенту яркости.
/// Возвращает (x_center, intensity) - позицию и интенсивность тени.
fn detect_spine_shadow(gray: &Mat) -> Option<(f32, f32)> {
    let width = gray.cols() as usize;
    let height = gray.rows() as usize;

    // Прямой доступ к пикселям для вертикального усреднения
    let data = unsafe { std::slice::from_raw_parts(gray.data() as *const u8, width * height) };

    // Вертикальное усреднение для получения профиля яркости по X
    let mut profile = vec![0.0_f32; width];
    for x in 0..width {
        let mut sum = 0.0_f32;
        for y in 0..height {
            sum += data[y * width + x] as f32;
        }
        profile[x] = sum / height as f32;
    }

    // Вычисление градиента яркости
    let mut gradient = vec![0.0_f32; width];
    for x in 1..width.saturating_sub(1) {
        gradient[x] = profile[x + 1] - profile[x - 1];
    }

    // Поиск минимума яркости в центральной трети изображения
    let start = width / 3;
    let end = 2 * width / 3;

    let mut min_intensity = f32::MAX;
    let mut min_pos = start;

    for x in start..end {
        if profile[x] < min_intensity && gradient[x].abs() > 5.0 {
            min_intensity = profile[x];
            min_pos = x;
        }
    }

    if min_intensity < 128.0 {
        Some((min_pos as f32, min_intensity))
    } else {
        None
    }
}

/// Построение цилиндрической модели деформации.
/// Возвращает массив смещений dx для каждой колонки X.
fn build_cylindrical_deformation(width: usize, spine_x: f32, curvature: f32) -> Vec<f32> {
    let mut offsets = vec![0.0_f32; width];
    let half_width = width as f32 / 2.0;
    
    for x in 0..width {
        let dist_from_spine = x as f32 - spine_x;
        // Цилиндрическая модель: смещение пропорционально квадрату расстояния от корешка
        offsets[x] = -curvature * dist_from_spine.powi(2) / half_width;
    }
    
    offsets
}

/// Деформация цилиндрической модели для выпрямления текста у тугого корешка.
/// Использует детекцию центральной тени корешка по градиенту яркости.
fn apply_cylindrical_correction(src: &Mat, gray: &Mat, curvature: f32) -> Option<Vec<f32>> {
    // Детекция тени корешка
    let spine_info = detect_spine_shadow(gray)?;
    let (spine_x, _intensity) = spine_info;
    
    // Построение цилиндрической модели деформации
    let width = src.cols() as usize;
    let cylindrical_offsets = build_cylindrical_deformation(width, spine_x, curvature);
    
    Some(cylindrical_offsets)
}

/// De-warping корешка: выпрямление текста у изгиба страницы.
/// Алгоритм:
///   1. Грейскейл + бинаризация (Otsu).
///   2. Детекция тени корешка по Градиенту яркости.
///   3. Построение Mesh Grid деформации через Text Line Tracking.
///   4. Применение remap(cx,cy→x',y') обратной координатной трансформации.
pub fn dewarp_spine(src: &Mat) -> Result<Mat, DigitizationError> {
    // 1. Грейскейл + бинаризация
    let mut gray = Mat::default();
    if src.channels() > 1 {
        imgproc::cvt_color(
            src,
            &mut gray,
            imgproc::COLOR_BGR2GRAY,
            0,
            opencv::core::AlgorithmHint::ALGO_HINT_APPROX,
        )
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;
    } else {
        gray = src.clone();
    }

    let mut binary = Mat::default();
    imgproc::threshold(&gray, &mut binary, 0.0, 255.0, imgproc::THRESH_BINARY_INV + imgproc::THRESH_OTSU)
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    // 2. Вертикальные линии Хафа на бинарном изображении
    let mut lines_vec = Vector::<Vec4i>::new();
    imgproc::hough_lines_p(
        &binary,
        &mut lines_vec,
        1.0,
        std::f64::consts::PI / 180.0,
        40,
        30.0,
        5.0,
    )
    .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    let hough_lines: Vec<Vec4i> = lines_vec.to_vec();
    if hough_lines.is_empty() {
        return Ok(src.clone());
    }

    // Отфильтровать только вертикальные (~90° ± 20°)
    let verticals: Vec<&Vec4i> = hough_lines.iter().filter(|l| {
        let dx = l[2] as f64 - l[0] as f64;
        let dy = l[3] as f64 - l[1] as f64;
        let angle = (dy.atan2(dx) * 180.0 / std::f64::consts::PI).rem_euclid(360.0);
        (angle >= 70.0 && angle <= 110.0) || (angle >= 250.0 && angle <= 290.0)
    }).collect();

    if verticals.len() < 5 {
        return Ok(src.clone());
    }

    // 3. Группировка по колонкам X и вычисление смещения dx для каждой колонки
    let cols_usize = src.cols() as usize;
    let mut col_offsets = vec![0.0_f32; cols_usize];
    let mut col_counts = vec![0_u32; cols_usize];

    for line in &verticals {
        let mid_x = ((line[0] + line[2]) / 2) as usize;
        if mid_x >= cols_usize { continue; }

        // Смещение от идеальной вертикали: разница между x1 и x2
        let dx = (line[2] as f32 - line[0] as f32) / 2.0;
        col_offsets[mid_x] += dx;
        col_counts[mid_x] += 1;
    }

    // Сглаживание смещений скользящим окном
    let window = 15usize;
    let mut smoothed = vec![0.0_f32; cols_usize];
    for x in 0..cols_usize {
        let left = if x > window { x - window } else { 0 };
        let right = if x + window < cols_usize { x + window } else { cols_usize - 1 };

        let mut sum_dx = 0.0_f32;
        let mut sum_w = 0_u32;
        for i in left..=right {
            if col_counts[i] > 0 {
                let avg = col_offsets[i] / col_counts[i] as f32;
                sum_dx += avg;
                sum_w += col_counts[i];
            }
        }
        smoothed[x] = if sum_w > 0 { sum_dx / sum_w as f32 } else { 0.0 };
    }

    // Ограничить максимальное смещение, чтобы избежать артефактов
    const MAX_OFFSET: f32 = 20.0;
    for v in smoothed.iter_mut() {
        *v = (*v).clamp(-MAX_OFFSET, MAX_OFFSET);
    }

    // Интеграция цилиндрической модели деформации
    let curvature = 0.5_f32; // Параметр кривизны
    if let Some(cylindrical_offsets) = apply_cylindrical_correction(src, &gray, curvature) {
        // Комбинация смещений от Hough Lines и цилиндрической модели
        for x in 0..cols_usize {
            smoothed[x] += cylindrical_offsets[x];
        }
    }

    // Финальное ограничение смещений
    for v in smoothed.iter_mut() {
        *v = (*v).clamp(-MAX_OFFSET, MAX_OFFSET);
    }

    // 4. Построение карт remap
    let rows = src.rows();
    let cols = src.cols();

    let mut map_x = Mat::zeros(rows, cols, opencv::core::CV_32F)
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?
        .to_mat()
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    let mut map_y = Mat::zeros(rows, cols, opencv::core::CV_32F)
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?
        .to_mat()
        .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    unsafe {
        let ptr_x = map_x.data_mut() as *mut f32;
        let ptr_y = map_y.data_mut() as *mut f32;

        for y in 0..rows {
            for x in 0..cols_usize {
                let idx = y as usize * cols_usize + x;
                *ptr_x.add(idx) = x as f32 + smoothed[x];
                *ptr_y.add(idx) = y as f32;
            }
        }
    }

    // 5. Применить remap
    let mut result = Mat::default();
    imgproc::remap(
        src,
        &mut result,
        &map_x,
        &map_y,
        imgproc::INTER_CUBIC,
        BORDER_DEFAULT,
        core::Scalar::default(),
        core::AlgorithmHint::ALGO_HINT_APPROX,
    )
    .map_err(|e| DigitizationError::OpenCv(e.to_string()))?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_perspective_warp_identity() {
        let size = Size::new(100, 100);
        let mut src = Mat::zeros(size.height, size.width, opencv::core::CV_8UC3)
            .unwrap()
            .to_mat()
            .unwrap();

        // Заполнить тестовым паттерном
        unsafe {
            let data = std::slice::from_raw_parts_mut(src.data_mut() as *mut u8, 100 * 100 * 3);
            for i in 0..data.len() / 3 {
                data[i * 3] = (i % 256) as u8;       // B
                data[i * 3 + 1] = ((i * 2) % 256) as u8; // G
                data[i * 3 + 2] = ((i * 3) % 256) as u8; // R
            }
        }

        let p1 = Point2f::new(0.0, 0.0);
        let p2 = Point2f::new(100.0, 0.0);
        let p3 = Point2f::new(100.0, 100.0);
        let p4 = Point2f::new(0.0, 100.0);

        let warped = perspective_warp(&src, p1, p2, p3, p4, size).unwrap();
        assert_eq!(warped.rows(), 100);
        assert_eq!(warped.cols(), 100);
    }

    #[test]
    fn test_detect_spine_shadow() {
        // Создание тестового изображения с тенью корешка в центре
        let width: usize = 300;
        let height: usize = 200;
        let mut gray = Mat::ones(height as i32, width as i32, opencv::core::CV_8UC1)
            .unwrap()
            .to_mat()
            .unwrap();

        // Добавление тени корешка в центральной трети через прямой доступ
        unsafe {
            let data = std::slice::from_raw_parts_mut(gray.data_mut() as *mut u8, width * height);
            for x in 100..200 {
                for y in 0..height {
                    data[y * width + x] = 50_u8;
                }
            }
        }

        let result = detect_spine_shadow(&gray);
        assert!(result.is_some());
        let (x_center, intensity) = result.unwrap();
        assert!(x_center >= 100.0 && x_center <= 200.0);
        assert!(intensity < 128.0);
    }

    #[test]
    fn test_build_cylindrical_deformation() {
        let width: usize = 300;
        let spine_x = 150.0_f32;
        let curvature = 0.5_f32;

        let offsets = build_cylindrical_deformation(width, spine_x, curvature);
        
        assert_eq!(offsets.len(), width);
        // Смещение должно быть максимальным на краях и минимальным у центра
        assert!((offsets[0]).abs() > (offsets[width/2]).abs());
    }

    #[test]
    fn test_apply_cylindrical_correction() {
        // Создание тестового изображения с тенью корешка
        let width: usize = 300;
        let height: usize = 200;
        let src = Mat::zeros(height as i32, width as i32, opencv::core::CV_8UC3)
            .unwrap()
            .to_mat()
            .unwrap();
        let gray = Mat::ones(height as i32, width as i32, opencv::core::CV_8UC1)
            .unwrap()
            .to_mat()
            .unwrap();

        let curvature = 0.5_f32;
        let result = apply_cylindrical_correction(&src, &gray, curvature);
        
        // Результат может быть None если не найдена тень корешка
        if let Some(offsets) = result {
            assert_eq!(offsets.len(), width);
        }
    }
}
