use opencv::core::{Mat, Point2f, Size};
pub mod binarization;
pub mod segmentation;
pub mod warping;

// Реэкспорт структур данных для чистоты вызовов в main.rs
pub use binarization::apply_sauvola_threshold;
pub use segmentation::{CustomPoint, PageVertices, process_book_contours, segment_pages};
pub use warping::{dewarp_spine, perspective_warp};

/// Полная коррекция страницы: перспективная трансформация + деварпинг корешка.
pub fn rectify_and_dewarp_page(src: &Mat, vertices: &PageVertices, target_width: u32, target_height: u32) -> Result<Mat, String> {
    let to_point = |cp: &CustomPoint| Point2f::new(cp.x as f32, cp.y as f32);

    // Перспективная трансформация
    let size = Size::new(target_width as i32, target_height as i32);
    let warped = perspective_warp(
        src,
        to_point(&vertices.p1),
        to_point(&vertices.p2),
        to_point(&vertices.p3),
        to_point(&vertices.p4),
        size,
    )?;

    // Деварпинг корешка
    dewarp_spine(&warped)
}