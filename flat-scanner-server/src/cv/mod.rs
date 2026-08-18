use opencv::core::{Mat, Point2f, Size};
pub mod binarization;
pub mod calibration;
pub mod ccitt_encoder;
pub mod profile_filtering;
pub mod seal_extraction;
pub mod segmentation;
pub mod warping;

/// Типизированная ошибка цифровизации страницы (TECH_SPEC_addon_1.md §1.1).
/// Возвращается всеми cv-функциями вместо свободных строк.
#[derive(Debug, thiserror::Error)]
pub enum DigitizationError {
    /// Геометрия страницы невалидна: не 4 точки, площадь контура < 15% кадра,
    /// контур вырожден или не выпуклый.
    #[error("invalid page geometry: {0}")]
    InvalidPageGeometry(String),

    /// Контур страницы не обнаружен на кадре.
    #[error("no page contour found: {0}")]
    NoContourFound(String),

    /// Контур вырожден (периметр/площадь ниже порога аппроксимации).
    #[error("degenerate contour: {0}")]
    DegenerateContour(String),

    /// Ошибка низкоуровневого вызова OpenCV.
    #[error("opencv error: {0}")]
    OpenCv(String),

    /// Ошибка подсистемы SANE FFI (TECH_SPEC_addon_2.md §4).
    #[error("Ошибка подсистемы SANE FFI: {0}")]
    SaneError(String),

    /// Исключение ядра OpenCV C++ (перехвачено) (TECH_SPEC_addon_2.md §4).
    #[error("Исключение ядра OpenCV C++ (Перехвачено): {0}")]
    OpenCVPanic(String),

    /// Ошибка транзакции базы данных SQLite (TECH_SPEC_addon_2.md §4).
    #[error("Ошибка транзакции базы данных SQLite: {0}")]
    DatabaseError(#[from] rusqlite::Error),

    /// Ошибка ввода-вывода файловой системы (TECH_SPEC_addon_2.md §4).
    #[error("Ошибка ввода-вывода файловой системы: {0}")]
    IoError(#[from] std::io::Error),
}

impl From<DigitizationError> for String {
    fn from(e: DigitizationError) -> Self {
        e.to_string()
    }
}

// Реэкспорт структур данных для чистоты вызовов в main.rs
pub use binarization::apply_sauvola_threshold;
pub use segmentation::{CustomPoint, PageVertices, process_book_contours, segment_pages, detect_skew_angle, rotate_image};
pub use warping::{dewarp_spine, perspective_warp, safe_calculate_homography, validate_page_geometry};
pub use ccitt_encoder::encode_ccitt_g4_to_file;
pub use profile_filtering::{apply_profile, ProcessingProfile};
pub use seal_extraction::{extract_seal_mask, overlay_seal_on_text};

/// Полная коррекция страницы: перспективная трансформация + деварпинг корешка.
pub fn rectify_and_dewarp_page(src: &Mat, vertices: &PageVertices, target_width: u32, target_height: u32) -> Result<Mat, DigitizationError> {
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