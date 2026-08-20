# TECH_SPEC_addon_2.md: Слой Безопасного Взаимодействия с OpenCV и SANE FFI

## 1. Назначение документа
Документ специфицирует требования к реализации безопасных, отказоустойчивых абстракций (Safe Wrappers) вокруг внешних Си/C++ интерфейсов (`sane-backends` и `opencv`). Исключает риски утечек памяти (Memory Leaks), порчи кучи (Heap Corruption) и аварийного завершения процесса (Abort/Panic) при обработке некорректных данных.

## 2. Модуль SANE FFI: Безопасный менеджмент ресурсов (RAII & Thread Safety)

### 2.1. Управление жизненным циклом (Трейт `Drop`)
Категорически запрещено оставлять закрытие Си-дескрипторов на усмотрение вызывающего кода. Модуль обязан инкапсулировать сырые Си-указатели в безопасные структуры Rust.

```rust
/// Безопасная обёртка над Си-дескриптором сканера SANE
pub struct SaneScanner {
    handle: *mut std::ffi::c_void, // Сырой Си-указатель на SANE_Handle
}

// Запрещаем небезопасное неявное копирование
impl !Copy for SaneScanner {}
impl !Clone for SaneScanner {}

/// Гарантированное освобождение ресурсов Си-библиотеки по стандарту RAII
impl Drop for SaneScanner {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                // Вызов Си-функции закрытия устройства из sane-backends
                sane_close(self.handle);
            }
        }
    }
}
```

### 2.2. Потоковая безопасность (Thread Safety)
Поскольку вызовы SANE выполняются внутри пула потоков `tokio::task::spawn_blocking`, структуры-обёртки должны явно декларировать возможность переноса между потоками (Thread Interoperability):

```rust
// Указываем компилятору, что структуру безопасно передавать в spawn_blocking
unsafe impl Send for SaneScanner {}
// Запрещаем одновременный доступ из нескольких потоков (SANE не потокобезопасна)
impl !Sync for SaneScanner {}
```

### 2.3. Алгоритм чтения данных (Снижение аллокаций)
*   Функция чтения данных должна принимать переиспользуемый буфер `&mut [u8]` вместо создания нового `Vec<u8>` на каждую итерацию `sane_read`. Это предотвращает фрагментацию кучи при обработке 16-битных тяжелых TIFF-кадров.

---

## 3. Модуль OpenCV Core: Изоляция C++ Исключений (Exception Guard)

### 3.1. Перехват C++ `cv::Exception`
Библиотека OpenCV написана на C++ и выбрасывает исключения `cv::Exception`. В случае геометрических аномалий (например, некорректные аргументы в `get_perspective_transform` или `warp_perspective`) необработанное C++ исключение разрушает стек Rust, вызывая моментальный `SIGABRT` (системный Abort).

*   **Контракт:** Все вызовы функций OpenCV должны быть обёрнуты в обработку типов `Result`, предоставляемую крейтом `opencv`.
*   **Запрет:** Использование `unwrap()`, `expect()` или игнорирование возвращаемых ошибок (`let _ = ...`) внутри графического конвейера **категорически запрещено**.

```rust
use opencv::core::{Mat, Point2f, Vector};
use opencv::imgproc;

/// Безопасный расчёт матрицы гомографии с изоляцией C++ исключений
pub fn safe_calculate_homography(
    src_points: &Vector<Point2f>,
    dst_points: &Vector<Point2f>
) -> Result<Mat, DigitizationError> {
    // Метод возвращает Result<Mat, opencv::Error>, перехватывая cv::Exception
    imgproc::get_perspective_transform(src_points, dst_points)
        .map_err(|opencv_err| {
            DigitizationError::OpenCVPanic(format!(
                "OpenCV C++ Exception во время расчета гомографии: {}", 
                opencv_err.message
            ))
        })
}
```

### 3.2. Геометрический Валидатор (Строгий контракт перед Warp)
Перед вызовом трансформации перспективы, модуль обработки обязан выполнить валидацию контура. Если валидация не пройдена, выполнение прерывается на уровне Rust, не допуская вызова Си-функций OpenCV.

```rust
pub fn validate_page_geometry(
    contour: &Vector<Point2f>, 
    frame_area: f64
) -> Result<(), DigitizationError> {
    // 1. Проверка количества опорных точек (строго четырёхугольник)
    if contour.len() != 4 {
        return Err(DigitizationError::InvalidPageGeometry(format!(
            "Контур содержит {} точек вместо строго 4", contour.len()
        )));
    }

    // 2. Проверка геометрической выпуклости фигуры (convexity)
    let is_convex = imgproc::is_contour_convex(contour)
        .map_err(|e| DigitizationError::OpenCVPanic(e.to_string()))?;
        
    if !is_convex {
        return Err(DigitizationError::InvalidPageGeometry(
            "Обнаружен вогнутый или самопересекающийся контур страницы".to_string()
        ));
    }

    // 3. Проверка площади контура (минимум 15% от общей площади матрицы сканера)
    let contour_area = imgproc::contour_area(contour, false)
        .map_err(|e| DigitizationError::OpenCVPanic(e.to_string()))?;
        
    if contour_area < (frame_area * 0.15) {
        return Err(DigitizationError::InvalidPageGeometry(format!(
            "Площадь контура ({:.2}) меньше критического порога в 15% от кадра", 
            contour_area
        )));
    }

    Ok(())
}
```

---

## 4. Спецификация Ошибок Ядра (`DigitizationError`)
Перечисление `enum DigitizationError` должно быть расширено для точной классификации низкоуровневых сбоев, передаваемых в фоновую очередь записи `SQLite`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum DigitizationError {
    #[error("Ошибка подсистемы SANE FFI: {0}")]
    SaneError(String),

    #[error("Критический сбой геометрии страницы: {0}")]
    InvalidPageGeometry(String),

    #[error("Исключение ядра OpenCV C++ (Перехвачено): {0}")]
    OpenCVPanic(String),

    #[error("Ошибка транзакции базы данных SQLite: {0}")]
    DatabaseError(#[from] rusqlite::Error),
    
    #[error("Ошибка ввода-вывода файловой системы: {0}")]
    IoError(#[from] std::io::Error),
}
```
