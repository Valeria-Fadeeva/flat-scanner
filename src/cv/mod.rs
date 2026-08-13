pub mod binarization;
pub mod segmentation;
pub mod warping;

// Реэкспорт структур данных для чистоты вызовов в main.rs
pub use binarization::apply_sauvola_threshold;
pub use segmentation::{PageVertices, process_book_contours};
pub use warping::{dewarp_spine, perspective_warp};
