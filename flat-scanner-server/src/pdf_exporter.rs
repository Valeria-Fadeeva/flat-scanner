//! G5: Сборка финального PDF из страниц книги.
//!
//! Читает TIFF/PNG-страницы (CCITT G4 или PNG) через `lopdf::xobject::image`
//! (внутренний декодер `image` crate, без системных зависимостей) и собирает
//! PDF-документ: каждая страница — Image XObject, растянутый на MediaBox.
//!
//! Порядок страниц задаёт вызывающий код (по `spread_index ASC`, внутри
//! разворота — сначала левая, затем правая страница).

use lopdf::xobject::image;
use lopdf::{Dictionary, Document, Object, Stream};

/// Метаданные PDF (заголовок, автор, тема).
#[derive(Debug, Clone)]
pub struct PdfMetadata {
    pub title: String,
    pub author: String,
    pub subject: String,
}

/// Собирает PDF из упорядоченного списка путей к страницам (TIFF/PNG).
///
/// Возвращает размер итогового PDF в байтах.
///
/// # Аргументы
/// - `page_paths` — упорядоченный список путей к страницам.
/// - `metadata` — метаданные PDF.
/// - `output_path` — путь к выходному PDF-файлу.
pub fn assemble_pdf_from_tiff_pages(
    page_paths: &[String],
    metadata: &PdfMetadata,
    output_path: &str,
) -> Result<usize, String> {
    if page_paths.is_empty() {
        return Err("Нет страниц для экспорта".to_string());
    }

    let mut doc = Document::with_version("1.5");

    // Корень дерева страниц (ID известен заранее — нужен как Parent для страниц).
    let pages_id = doc.new_object_id();
    let mut kids: Vec<Object> = Vec::with_capacity(page_paths.len());

    for path in page_paths {
        // Декодируем изображение в Image XObject (lopdf сам определяет
        // формат, размер и цветовое пространство).
        let img_stream = image(path).map_err(|e| format!("image({}): {}", path, e))?;

        let width = img_stream
            .dict
            .get(b"Width")
            .and_then(|o| o.as_i64())
            .map_err(|e| format!("Width({}): {}", path, e))?;
        let height = img_stream
            .dict
            .get(b"Height")
            .and_then(|o| o.as_i64())
            .map_err(|e| format!("Height({}): {}", path, e))?;

        if width <= 0 || height <= 0 {
            return Err(format!("Некорректный размер страницы: {}", path));
        }

        let xref = doc.add_object(img_stream);

        // Content stream: растягивает изображение на всю страницу.
        let content = format!("q {} 0 0 {} 0 0 cm /img Do Q", width, height);
        let content_id =
            doc.add_object(Stream::new(Dictionary::new(), content.into_bytes()));

        // Resources: регистрируем XObject под именем /img.
        let mut xobjects = Dictionary::new();
        xobjects.set("img", xref);
        let mut resources = Dictionary::new();
        resources.set("XObject", xobjects);
        let resources_id = doc.add_object(resources);

        // Страница: Type=Page, Parent, Contents, Resources, MediaBox.
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

        kids.push(page_id.into());
    }

    // Дерево страниц.
    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", kids);
    pages.set("Count", page_paths.len() as i64);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    // Метаданные (Info dict).
    let mut info = Dictionary::new();
    info.set("Title", metadata.title.clone());
    info.set("Author", metadata.author.clone());
    info.set("Subject", metadata.subject.clone());
    let info_id = doc.add_object(info);

    // Каталог документа.
    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    catalog.set("Info", info_id);
    let catalog_id = doc.add_object(catalog);

    doc.trailer.set("Root", catalog_id);
    doc.compress();
    doc.save(output_path).map_err(|e| format!("save: {}", e))?;

    let size = std::fs::metadata(output_path)
        .map_err(|e| format!("metadata: {}", e))?
        .len() as usize;

    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> PdfMetadata {
        PdfMetadata {
            title: "Test".to_string(),
            author: "Author".to_string(),
            subject: "Subject".to_string(),
        }
    }

    #[test]
    fn test_metadata_clone() {
        let m = meta();
        let c = m.clone();
        assert_eq!(c.title, "Test");
        assert_eq!(c.author, "Author");
        assert_eq!(c.subject, "Subject");
    }

    #[test]
    fn test_empty_pages_returns_error() {
        let result = assemble_pdf_from_tiff_pages(&[], &meta(), "/tmp/test_empty.pdf");
        assert!(result.is_err());
    }

    #[test]
    fn test_nonexistent_page_returns_error() {
        let result = assemble_pdf_from_tiff_pages(
            &["/nonexistent/page.tiff".to_string()],
            &meta(),
            "/tmp/test_missing.pdf",
        );
        assert!(result.is_err());
    }
}