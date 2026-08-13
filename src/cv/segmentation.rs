use opencv::{
    core::{self, BORDER_DEFAULT, Mat, Point, Point2f, Size, Vec4i, Vector},
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

/// Основная функция детекции разворота книги на кадре сканера.
/// Использует HoughLinesP + кластеризацию углов → пересечение прямых → P₁..P₄.
/// Fallback: findContours + approxPolyDP / minAreaRect.
pub fn process_book_contours(src: &Mat) -> Result<PageVertices, String> {
    let mut gray = Mat::default();
    let mut blurred = Mat::default();
    let mut edges = Mat::default();

    // 1. Предобработка
    imgproc::cvt_color(
        src,
        &mut gray,
        imgproc::COLOR_BGR2GRAY,
        0,
        core::AlgorithmHint::ALGO_HINT_APPROX,
    )
    .map_err(|e| e.to_string())?;

    imgproc::gaussian_blur(
        &gray,
        &mut blurred,
        Size::new(7, 7),
        0.0,
        0.0,
        BORDER_DEFAULT,
        core::AlgorithmHint::ALGO_HINT_APPROX,
    )
    .map_err(|e| e.to_string())?;

    imgproc::canny(&blurred, &mut edges, 50.0, 150.0, 3, false).map_err(|e| e.to_string())?;

    // 2. Прямые Хафа
    let mut lines_vec = Vector::<Vec4i>::new();
    imgproc::hough_lines_p(
        &edges,
        &mut lines_vec,
        1.0,
        std::f64::consts::PI / 180.0,
        80,
        50.0,
        10.0,
    )
    .map_err(|e| e.to_string())?;

    let hough_lines: Vec<Vec4i> = lines_vec.to_vec();
    if hough_lines.is_empty() {
        return detect_vertices_by_contours(src, &gray);
    }

    // 3. Кластеризация по углам
    let (top_lines, bottom_lines, left_lines, right_lines) = cluster_hough_lines(&hough_lines, src);

    // 4. Попытка восстановить вершины через пересечение репрезентативных прямых
    if let Some(vertices) = compute_vertices_from_clusters(&top_lines, &bottom_lines, &left_lines, &right_lines) {
        return Ok(vertices);
    }

    // Fallback на контурный поиск
    detect_vertices_by_contours(src, &gray)
}

/// Разделение отрезков Хафа на четыре группы по углу и позиции
fn cluster_hough_lines<'a>(lines: &'a [Vec4i], src: &'a Mat) -> (Vec<&'a Vec4i>, Vec<&'a Vec4i>, Vec<&'a Vec4i>, Vec<&'a Vec4i>) {
    let mut top_lines: Vec<&Vec4i> = Vec::new();
    let mut bottom_lines: Vec<&Vec4i> = Vec::new();
    let mut left_lines: Vec<&Vec4i> = Vec::new();
    let mut right_lines: Vec<&Vec4i> = Vec::new();

    for line in lines.iter() {
        let dx = line[2] as f64 - line[0] as f64;
        let dy = line[3] as f64 - line[1] as f64;
        let angle = (dy.atan2(dx) * 180.0 / std::f64::consts::PI).rem_euclid(360.0);

        // Горизонтальные (~0° или ~180°)
        if angle < 25.0 || angle > 155.0 {
            let mid_y = (line[1] + line[3]) / 2;
            if mid_y < src.rows() / 2 {
                top_lines.push(line);
            } else {
                bottom_lines.push(line);
            }
        }
        // Вертикальные (~90°)
        else if angle >= 65.0 && angle <= 115.0 {
            let mid_x = (line[0] + line[2]) / 2;
            if mid_x < src.cols() / 2 {
                left_lines.push(line);
            } else {
                right_lines.push(line);
            }
        }
    }

    (top_lines, bottom_lines, left_lines, right_lines)
}

/// Получить одну репрезентативную прямую из кластера отрезков (взвешенная медиана)
fn representative_line(lines: &[&Vec4i]) -> Option<(Point, Point)> {
    if lines.is_empty() {
        return None;
    }

    let mut total_len = 0.0_f64;
    let mut sum_x1 = 0.0_f64;
    let mut sum_y1 = 0.0_f64;
    let mut sum_x2 = 0.0_f64;
    let mut sum_y2 = 0.0_f64;

    for l in lines {
        let dx = l[2] as f64 - l[0] as f64;
        let dy = l[3] as f64 - l[1] as f64;
        let len = (dx * dx + dy * dy).sqrt();
        total_len += len;
        sum_x1 += l[0] as f64 * len;
        sum_y1 += l[1] as f64 * len;
        sum_x2 += l[2] as f64 * len;
        sum_y2 += l[3] as f64 * len;
    }

    if total_len < 1.0 {
        return None;
    }

    let cx1 = sum_x1 / total_len;
    let cy1 = sum_y1 / total_len;
    let cx2 = sum_x2 / total_len;
    let cy2 = sum_y2 / total_len;

    let dir_x = cx2 - cx1;
    let dir_y = cy2 - cy1;
    let norm = (dir_x * dir_x + dir_y * dir_y).sqrt();
    if norm < 1e-6 {
        return None;
    }

    // Экстраполируем линию далеко за пределы изображения для надёжного пересечения
    let scale = 5000.0 / norm;
    let ext_x = dir_x * scale;
    let ext_y = dir_y * scale;

    Some((
        Point::new((cx1 - ext_x) as i32, (cy1 - ext_y) as i32),
        Point::new((cx1 + ext_x) as i32, (cy1 + ext_y) as i32),
    ))
}

/// Точка пересечения двух прямых (p1-p2) и (p3-p4)
fn line_intersection(p1: Point, p2: Point, p3: Point, p4: Point) -> Option<Point> {
    let x1 = p1.x as f64;
    let y1 = p1.y as f64;
    let x2 = p2.x as f64;
    let y2 = p2.y as f64;
    let x3 = p3.x as f64;
    let y3 = p3.y as f64;
    let x4 = p4.x as f64;
    let y4 = p4.y as f64;

    let denom = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4);
    if denom.abs() < 1e-6 {
        return None; // параллельны
    }

    let t = ((x1 - x3) * (y3 - y4) - (y1 - y3) * (x3 - x4)) / denom;
    let ix = (x1 + t * (x2 - x1)).round() as i32;
    let iy = (y1 + t * (y2 - y1)).round() as i32;
    Some(Point::new(ix, iy))
}

/// Восстановить вершины из четырёх кластеров линий
fn compute_vertices_from_clusters(
    top_lines: &[&Vec4i],
    bottom_lines: &[&Vec4i],
    left_lines: &[&Vec4i],
    right_lines: &[&Vec4i],
) -> Option<PageVertices> {
    let tl = representative_line(top_lines)?;
    let bl = representative_line(bottom_lines)?;
    let ll = representative_line(left_lines)?;
    let rl = representative_line(right_lines)?;

    let p1 = line_intersection(tl.0, tl.1, ll.0, ll.1)?; // верх × лево
    let p2 = line_intersection(tl.0, tl.1, rl.0, rl.1)?; // верх × право
    let p3 = line_intersection(bl.0, bl.1, rl.0, rl.1)?; // низ × право
    let p4 = line_intersection(bl.0, bl.1, ll.0, ll.1)?; // низ × лево

    Some(PageVertices {
        p1: CustomPoint { x: p1.x, y: p1.y },
        p2: CustomPoint { x: p2.x, y: p2.y },
        p3: CustomPoint { x: p3.x, y: p3.y },
        p4: CustomPoint { x: p4.x, y: p4.y },
    })
}

/// Fallback: детекция через findContours → approxPolyDP / minAreaRect
fn detect_vertices_by_contours(src: &Mat, gray: &Mat) -> Result<PageVertices, String> {
    let mut thresh = Mat::default();
    imgproc::threshold(gray, &mut thresh, 0.0, 255.0, imgproc::THRESH_BINARY_INV + imgproc::THRESH_OTSU)
        .map_err(|e| e.to_string())?;

    let mut contours_vec = Vector::<Vector::<Point>>::new();
    imgproc::find_contours(&thresh, &mut contours_vec, imgproc::RETR_EXTERNAL, imgproc::CHAIN_APPROX_SIMPLE, Point::new(0, 0))
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