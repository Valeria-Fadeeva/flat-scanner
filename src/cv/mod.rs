pub mod binarization;
pub mod segmentation;

// Реэкспорт структур данных для чистоты вызовов в main.rs
pub use binarization::apply_sauvola_threshold;
pub use segmentation::{PageVertices, process_book_contours};
