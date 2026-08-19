//! Axum HTTP routes for the scan API (TECH_SPEC_addon_4.md).
//!
//! Provides `POST /api/v1/scan` endpoint that triggers the full
//! digitization pipeline: SANE capture → geometry → warp → binarize → save.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::cv::DigitizationError;
use crate::pipeline::PageProcessor;
use crate::sane_core;

/// Request body for POST /api/v1/scan.
#[derive(Debug, Deserialize)]
pub struct ScanRequest {
    pub book_id: String,
    pub page_number: i32,
}

/// Maps `DigitizationError` variants to appropriate HTTP status codes.
fn digitization_error_to_response(err: &DigitizationError) -> Response {
    let (status, code) = match err {
        DigitizationError::SaneError(_) => (StatusCode::SERVICE_UNAVAILABLE, "SCANNER_UNAVAILABLE"),
        DigitizationError::InvalidPageGeometry(_)
        | DigitizationError::NoContourFound(_)
        | DigitizationError::DegenerateContour(_) => (StatusCode::UNPROCESSABLE_ENTITY, "BAD_PAGE_GEOMETRY"),
        DigitizationError::OpenCv(_)
        | DigitizationError::OpenCVPanic(_)
        | DigitizationError::DatabaseError(_)
        | DigitizationError::IoError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
    };

    (
        status,
        Json(serde_json::json!({
            "status": "FAILED",
            "code": code,
            "message": err.to_string(),
        })),
    )
        .into_response()
}

/// POST /api/v1/scan — triggers full page digitization pipeline.
///
/// 1. Validates scanner availability (503 on failure).
/// 2. Spawns the blocking pipeline in `spawn_blocking`.
/// 3. Returns 200 on success, mapped error status on failure.
pub async fn handle_scan(
    axum::Extension(processor): axum::Extension<Arc<PageProcessor>>,
    Json(payload): Json<ScanRequest>,
) -> Response {
    // 1. Scanner availability check (RAII: dropped immediately, only validates hardware)
    let device_name = match sane_core::detect_hardware_scanner() {
        Ok(name) => name,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "ERROR",
                    "code": "SCANNER_UNAVAILABLE",
                    "message": format!("Сканер недоступен: {}", e),
                })),
            )
                .into_response();
        }
    };

    // Verify SaneScanner can be instantiated (catches busy/offline devices)
    match sane_core::SaneScanner::new(&device_name) {
        Ok(_scanner) => {} // dropped here, releases device
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "ERROR",
                    "code": "SCANNER_BUSY",
                    "message": format!("Сканер занят или отключён: {}", e),
                })),
            )
                .into_response();
        }
    }

    // 2. Spawn blocking pipeline
    let book_id = payload.book_id.clone();
    let book_id_for_task = book_id.clone();
    let device = device_name.clone();
    let processor = processor.clone();

    let result = tokio::task::spawn_blocking(move || {
        processor.process_page(&book_id_for_task, None, &device)
    })
    .await;

    // 3. Handle spawn_blocking JoinError (task panicked)
    let page_result = match result {
        Ok(inner) => inner,
        Err(join_err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "status": "FAILED",
                    "code": "TASK_PANIC",
                    "message": format!("Panic в spawn_blocking: {:?}", join_err),
                })),
            )
                .into_response();
        }
    };

    // 4. Map pipeline result to HTTP response
    match page_result {
        Ok(pr) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "SUCCESS",
                "message": "Страница успешно обработана и закоммичена в WAL",
                "book_id": book_id,
                "page_number": payload.page_number,
                "left_path": pr.left_path,
                "right_path": pr.right_path,
                "execution_time_ms": pr.execution_time_ms,
            })),
        )
            .into_response(),
        Err(e) => {
            // pipeline::process_page returns Result<PageResult, String>
            // Map known error patterns to appropriate statuses
            let err_str = e.to_lowercase();
            let (status, code) = if err_str.contains("sane") || err_str.contains("сканер") {
                (StatusCode::SERVICE_UNAVAILABLE, "SCANNER_ERROR")
            } else if err_str.contains("geometry")
                || err_str.contains("contour")
                || err_str.contains("контур")
            {
                (StatusCode::UNPROCESSABLE_ENTITY, "BAD_PAGE_GEOMETRY")
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR")
            };

            (
                status,
                Json(serde_json::json!({
                    "status": "FAILED",
                    "code": code,
                    "message": e,
                })),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_request_deserialize() {
        let json = r#"{"book_id": "9a7b1c3d-e5f6-4a3b-8c2d-1e0f2a3b4c5d", "page_number": 14}"#;
        let req: ScanRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.book_id, "9a7b1c3d-e5f6-4a3b-8c2d-1e0f2a3b4c5d");
        assert_eq!(req.page_number, 14);
    }

    #[test]
    fn test_scan_request_missing_fields() {
        let json = r#"{"book_id": "abc"}"#;
        let result: Result<ScanRequest, _> = serde_json::from_str(json);
        assert!(result.is_err(), "page_number is required");
    }

    #[test]
    fn test_error_mapping_sane() {
        let err = DigitizationError::SaneError("device busy".to_string());
        let resp = digitization_error_to_response(&err);
        let status = resp.status();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn test_error_mapping_geometry() {
        let err = DigitizationError::InvalidPageGeometry("area too small".to_string());
        let resp = digitization_error_to_response(&err);
        let status = resp.status();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn test_error_mapping_internal() {
        let err = DigitizationError::OpenCVPanic("segfault".to_string());
        let resp = digitization_error_to_response(&err);
        let status = resp.status();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}