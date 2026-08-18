# TECH_SPEC_addon_3.md: Сквозной Скоростной Пайплайн Обработки Кадров

## 1. Назначение документа
Документ задает жесткий контракт на сборку сквозного, неблокирующего конвейера (Pipeline) оцифровки. Цель — максимальное сокращение времени удержания аппаратного сканера и мгновенный сброс выпрямленного ч/б кадра на диск.

## 2. Архитектура Пайплайна (Однопоточный Исполнитель)
Все этапы обработки одной страницы выполняются последовательно внутри одного `tokio::task::spawn_blocking`, чтобы исключить накладные расходы на синхронизацию между потоками.

```
[SANE Capture] ──(Vec)──> [OpenCV Geometry & Warp] ──(Mat)──> [Fast Binarization] ──> [Disk Save & SQL Commit]
```

## 3. Этап: Адаптивная Бинаризация (OpenCV Native Fast)
Вместо ресурсоемких кастомных алгоритмов, модуль использует нативный, оптимизированный Си-метод `adaptive_threshold` из OpenCV. Он эффективно убирает тени от разворота книги на уровне графического ядра.

```rust
use opencv::core::{Mat, BORDER_REPLICATE};
use opencv::imgproc;

/// Мгновенная бинаризация для текста. Убирает серый фон и замятия.
pub fn fast_binarize(src: &Mat) -> Result<Mat, DigitizationError> {
    let mut gray = Mat::default();
    let mut dst = Mat::default();

    // 1. Принудительный перевод в градации серого, если кадр цветной
    if src.channels() == 3 {
        imgproc::cvt_color(src, &mut gray, imgproc::COLOR_BGR2GRAY, 0)
            .map_err(|e| DigitizationError::OpenCVPanic(e.to_string()))?;
    } else {
        gray = src.clone();
    }

    // 2. Адаптивный трешолд (Метод среднего в блоке)
    // Параметры 11 и 2 идеальны для отсечения теней А3 сканера без потери букв
    imgproc::adaptive_threshold(
        &gray,
        &mut dst,
        255.0,
        imgproc::ADAPTIVE_THRESH_MEAN_C,
        imgproc::THRESH_BINARY,
        11, // Размер адаптивного окна (block size)
        2.0  // Константа вычитания из среднего
    ).map_err(|e| DigitizationError::OpenCVPanic(e.to_string()))?;

    Ok(dst)
}
```

## 4. Сквозная Сборка Конвейера (Функция `process_page`)

Модель обязана реализовать главный рабочий метод в `src/pipeline.rs`:

```rust
use std::path::PathBuf;

pub struct PageProcessor {
    write_queue_tx: tokio::sync::mpsc::Sender<WriteTask>, // Канал к SQLite очереди
    output_dir: PathBuf,
}

impl PageProcessor {
    /// Главный асинхронный эндпоинт запуска сканирования страницы
    pub async fn process_page(
        &self,
        book_id: String,
        page_number: i32,
        mut scanner: SaneScanner
    ) -> Result<(), DigitizationError> {
        let tx = self.write_queue_tx.clone();
        let out_dir = self.output_dir.clone();

        // Уводим всю тяжелую цепочку в пул блокирующих потоков
        tokio::task::spawn_blocking(move || {
            // 1. Аллокация буфера и захват кадра из SANE
            let mut raw_buffer = Vec::with_capacity(8 * 1024 * 1024); // 8MB пред-аллокация
            scanner.read_frame(&mut raw_buffer)?;
            
            // Превращаем сырой буфер в матрицу OpenCV
            let raw_mat = opencv::imgcodecs::imdecode(&opencv::core::Vector::from_slice(&raw_buffer), opencv::imgcodecs::IMREAD_COLOR)
                .map_err(|e| DigitizationError::OpenCVPanic(e.to_string()))?;

            // 2. Поиск контуров (Page Detection)
            let contours = super::warping::find_page_contours(&raw_mat)?;
            let frame_area = (raw_mat.cols() * raw_mat.rows()) as f64;
            
            // 3. Геометрический щит (TECH_SPEC_addon_2.md)
            super::warping::validate_page_geometry(&contours, frame_area)?;

            // 4. Трансформация перспективы (Warp)
            let dewarped_mat = super::warping::safe_calculate_homography_and_warp(&raw_mat, &contours)?;

            // 5. Скоростная бинаризация (Ч/Б текст)
            let final_binary = fast_binarize(&dewarped_mat)?;

            // 6. Сохранение на диск (Сжатый PNG)
            let file_name = format!("{}_{}.png", book_id, page_number);
            let target_path = out_dir.join(&file_name);
            
            opencv::imgcodecs::imwrite(target_path.to_str().unwrap(), &final_binary, &opencv::core::Vector::new())
                .map_err(|e| DigitizationError::OpenCVPanic(e.to_string()))?;

            // 7. Мгновенная отправка статуса в SQLite Очередь (Атомарный коммит)
            tx.blocking_send(WriteTask::UpdatePage {
                book_id,
                page_number,
                raw_path: "STORED_IN_MEMORY".to_string(),
                dewarped_path: target_path.to_str().unwrap().to_string(),
                status: "DEWARPED".to_string(),
                error: None,
            }).map_err(|_| DigitizationError::SaneError("Сбой канала очереди записи".to_string()))?;

            Ok(())
        }).await.map_err(|_| DigitizationError::SaneError("Сбой Tokio runtime во время обработки".to_string()))?
    }
}
```
