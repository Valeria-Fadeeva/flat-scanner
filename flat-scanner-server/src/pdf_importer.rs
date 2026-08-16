//! G4: Разборка сторонних PDF.
//!
//! Открывает существующий PDF, экспортирует страницы как растровые слои
//! (через `pdftoppm`), поддерживает замену/вставку страниц и очистку
//! от шума сторонних сканов (через `cv::profile_filtering::apply_profile`).
//!
//! Структурные операции (замена/вставка) выполняются через `lopdf` —
//! чистый Rust, без внешних зависимостей. Растеризация страниц — через
//! `pdftoppm` (poppler-utils), т.к. `lopdf` не рендерит страницы.

use lopdf::xobject::image;
use lopdf::{Dictionary, Document, Object, Stream};
use std::path::Path;
use std::process::Command;

/// Экспортирует все страницы PDF как PNG-файлы в `output_dir`.
///
/// Использует `pdftoppm -png -r <dpi>`. Возвращает упорядоченный список
/// путей к созданным PNG-файлам (по номеру страницы).
///
/// # Аргументы
/// - `input_pdf` — путь к входному PDF.
/// - `output_dir` — каталог для PNG-файлов (создаётся, если не существует).
/// - `dpi` — разрешение растеризации (300/600).
pub fn import_pdf_pages(
    input_pdf: &str,
    output_dir: &str,
    dpi: u32,
) -> Result<Vec<String>, String> {
    if !Path::new(input_pdf).exists() {
        return Err(format!("PDF не найден: {}", input_pdf));
    }
    if dpi < 72 || dpi > 1200 {
        return Err(format!("Некорректное DPI: {}", dpi));
    }

    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("create_dir_all: {}", e))?;

    // pdftoppm -png -r <dpi> <input> <prefix> — создаёт <prefix>-1.png, <prefix>-2.png, ...
    let prefix = format!("{}/page", output_dir);
    let output = Command::new("pdftoppm")
        .args(["-png", "-r", &dpi.to_string(), input_pdf, &prefix])
        .output()
        .map_err(|e| format!("pdftoppm: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("pdftoppm failed: {}", stderr));
    }

    // Собираем созданные файлы по порядку
    let mut pages: Vec<String> = Vec::new();
    let mut i = 1u32;
    loop {
        let path = format!("{}-{}.png", prefix, i);
        if Path::new(&path).exists() {
            pages.push(path);
            i += 1;
        } else {
            break;
        }
    }

    if pages.is_empty() {
        return Err("Не удалось экспортировать ни одной страницы".to_string());
    }

    Ok(pages)
}

/// Заменяет страницу `page_index` (0-based) на изображение `replacement_image`.
///
/// Сохраняет результат в `output_pdf`. Остальные страницы не изменяются.
pub fn replace_page(
    input_pdf: &str,
    page_index: usize,
    replacement_image: &str,
    output_pdf: &str,
) -> Result<usize, String> {
    let mut doc = Document::load(input_pdf)
        .map_err(|e| format!("load: {}", e))?;

    let page_id = find_page_by_index(&doc, page_index)?;

    // Декодируем новое изображение в Image XObject
    let img_stream = image(replacement_image)
        .map_err(|e| format!("image({}): {}", replacement_image, e))?;

    let width = img_stream
        .dict
        .get(b"Width")
        .and_then(|o| o.as_i64())
        .map_err(|e| format!("Width: {}", e))?;
    let height = img_stream
        .dict
        .get(b"Height")
        .and_then(|o| o.as_i64())
        .map_err(|e| format!("Height: {}", e))?;

    if width <= 0 || height <= 0 {
        return Err("Некорректный размер изображения".to_string());
    }

    let xref = doc.add_object(img_stream);

    // Content stream: растягивает изображение на всю страницу
    let content = format!("q {} 0 0 {} 0 0 cm /img Do Q", width, height);
    let content_id =
        doc.add_object(Stream::new(Dictionary::new(), content.into_bytes()));

    // Resources: регистрируем XObject под именем /img
    let mut xobjects = Dictionary::new();
    xobjects.set("img", xref);
    let mut resources = Dictionary::new();
    resources.set("XObject", xobjects);
    let resources_id = doc.add_object(resources);

    // Обновляем страницу
    let page_dict = doc
        .get_object(page_id)
        .map_err(|e| format!("get_object: {}", e))?;
    if let Object::Dictionary(d) = page_dict {
        let mut new_dict = d.clone();
        new_dict.set("Contents", content_id);
        new_dict.set("Resources", resources_id);
        new_dict.set(
            "MediaBox",
            vec![0.into(), 0.into(), width.into(), height.into()],
        );
        doc.objects.insert(page_id, Object::Dictionary(new_dict));
    } else {
        return Err("Страница не является Dictionary".to_string());
    }

    doc.compress();
    doc.save(output_pdf).map_err(|e| format!("save: {}", e))?;

    let size = std::fs::metadata(output_pdf)
        .map_err(|e| format!("metadata: {}", e))?
        .len() as usize;

    Ok(size)
}

/// Вставляет новую страницу после `after_index` (0-based; -1 = в начало).
///
/// Возвращает размер итогового PDF в байтах.
pub fn insert_page(
    input_pdf: &str,
    after_index: i64,
    image_path: &str,
    output_pdf: &str,
) -> Result<usize, String> {
    let mut doc = Document::load(input_pdf)
        .map_err(|e| format!("load: {}", e))?;

    // Находим Pages-узел
    let pages_id = find_pages_root(&doc)?;

    // Декодируем изображение
    let img_stream = image(image_path)
        .map_err(|e| format!("image({}): {}", image_path, e))?;

    let width = img_stream
        .dict
        .get(b"Width")
        .and_then(|o| o.as_i64())
        .map_err(|e| format!("Width: {}", e))?;
    let height = img_stream
        .dict
        .get(b"Height")
        .and_then(|o| o.as_i64())
        .map_err(|e| format!("Height: {}", e))?;

    if width <= 0 || height <= 0 {
        return Err("Некорректный размер изображения".to_string());
    }

    let xref = doc.add_object(img_stream);

    let content = format!("q {} 0 0 {} 0 0 cm /img Do Q", width, height);
    let content_id =
        doc.add_object(Stream::new(Dictionary::new(), content.into_bytes()));

    let mut xobjects = Dictionary::new();
    xobjects.set("img", xref);
    let mut resources = Dictionary::new();
    resources.set("XObject", xobjects);
    let resources_id = doc.add_object(resources);

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("Contents", content_id);
    page.set("Resources", resources_id);
    page.set(
        "MediaBox",
        vec![0.into(), 0.into(), width.into(), height.into()],
    );
    let page_id = doc.add_object(page);

    // Обновляем Kids и Count в Pages-узле
    let pages_dict = doc
        .get_object(pages_id)
        .map_err(|e| format!("get_object: {}", e))?;
    if let Object::Dictionary(d) = pages_dict {
        let mut new_dict = d.clone();
        let mut kids: Vec<Object> = match new_dict.get(b"Kids") {
            Ok(Object::Array(arr)) => arr.clone(),
            _ => Vec::new(),
        };

        let insert_pos = if after_index < 0 {
            0
        } else {
            (after_index as usize).min(kids.len())
        };
        kids.insert(insert_pos, page_id.into());
        let count = kids.len() as i64;
        new_dict.set("Kids", kids);
        new_dict.set("Count", count);
        doc.objects.insert(pages_id, Object::Dictionary(new_dict));
    } else {
        return Err("Pages-узел не является Dictionary".to_string());
    }

    doc.compress();
    doc.save(output_pdf).map_err(|e| format!("save: {}", e))?;

    let size = std::fs::metadata(output_pdf)
        .map_err(|e| format!("metadata: {}", e))?
        .len() as usize;

    Ok(size)
}

/// Очищает страницу от шума сторонних сканов, применяя профиль обработки.
///
/// Возвращает путь к очищенному PNG-файлу.
pub fn clean_page(
    image_path: &str,
    profile: &str,
    k_factor: f32,
    window_size: i32,
) -> Result<String, String> {
    use crate::cv::profile_filtering::{apply_profile, ProcessingProfile};

    let mat = opencv::imgcodecs::imread(image_path, opencv::imgcodecs::IMREAD_COLOR)
        .map_err(|e| format!("imread({}): {}", image_path, e))?;

    let processing_profile = match profile.to_ascii_lowercase().as_str() {
        "text_bw_1bit" | "text" | "bw" | "1bit" => ProcessingProfile::TextBw1bit,
        "illustration_grayscale_8bit" | "illustration" | "gray" | "8bit" => {
            ProcessingProfile::IllustrationGrayscale8bit
        }
        "color_rgb_24bit" | "color" | "rgb" | "24bit" => ProcessingProfile::ColorRgb24bit,
        other => return Err(format!("Неизвестный профиль: {}", other)),
    };

    let cleaned = apply_profile(&mat, processing_profile, k_factor, window_size)
        .map_err(|e| format!("apply_profile: {}", e))?;

    let output_path = format!(
        "{}.clean.png",
        Path::new(image_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("page")
    );
    let params = opencv::core::Vector::default();
    opencv::imgcodecs::imwrite(&output_path, &cleaned, &params)
        .map_err(|e| format!("imwrite: {}", e))?;

    Ok(output_path)
}

/// Находит ID страницы по индексу (0-based) через обход дерева Pages.
fn find_page_by_index(doc: &Document, page_index: usize) -> Result<lopdf::ObjectId, String> {
    let pages_id = find_pages_root(doc)?;
    let pages = doc
        .get_object(pages_id)
        .map_err(|e| format!("get_object: {}", e))?;

    let kids = match pages {
        Object::Dictionary(d) => match d.get(b"Kids") {
            Ok(Object::Array(arr)) => arr.clone(),
            _ => return Err("Kids не найден".to_string()),
        },
        _ => return Err("Pages не Dictionary".to_string()),
    };

    if page_index >= kids.len() {
        return Err(format!(
            "Страница {} не найдена (всего {})",
            page_index,
            kids.len()
        ));
    }

    match &kids[page_index] {
        Object::Reference(id) => Ok(*id),
        _ => Err("Страница не Reference".to_string()),
    }
}

/// Находит корневой Pages-узел через Catalog.
fn find_pages_root(doc: &Document) -> Result<lopdf::ObjectId, String> {
    let catalog_id = doc
        .trailer
        .get(b"Root")
        .and_then(|o| o.as_reference())
        .map_err(|e| format!("Root: {}", e))?;

    let catalog = doc
        .get_object(catalog_id)
        .map_err(|e| format!("get_object: {}", e))?;

    match catalog {
        Object::Dictionary(d) => d
            .get(b"Pages")
            .and_then(|o| o.as_reference())
            .map_err(|e| format!("Pages: {}", e)),
        _ => Err("Catalog не Dictionary".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_import_nonexistent_pdf_returns_error() {
        let result = import_pdf_pages("/nonexistent.pdf", "/tmp/test_import", 300);
        assert!(result.is_err());
    }

    #[test]
    fn test_import_invalid_dpi_returns_error() {
        let result = import_pdf_pages("/tmp/test.pdf", "/tmp/test_import", 10);
        assert!(result.is_err());
    }

    #[test]
    fn test_replace_nonexistent_pdf_returns_error() {
        let result = replace_page(
            "/nonexistent.pdf",
            0,
            "/nonexistent.png",
            "/tmp/test_replace.pdf",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_insert_nonexistent_pdf_returns_error() {
        let result = insert_page(
            "/nonexistent.pdf",
            0,
            "/nonexistent.png",
            "/tmp/test_insert.pdf",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_clean_nonexistent_image_returns_error() {
        let result = clean_page("/nonexistent.png", "text_bw_1bit", 1.0, 15);
        assert!(result.is_err());
    }

    #[test]
    fn test_clean_unknown_profile_returns_error() {
        let result = clean_page("/nonexistent.png", "unknown_profile", 1.0, 15);
        assert!(result.is_err());
    }
}