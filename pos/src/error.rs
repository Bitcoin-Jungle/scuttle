use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;


/// The body of every error response.
#[derive(utoipa::ToSchema)]
#[allow(unused)]
pub struct ErrorBody {
	/// What went wrong, for people. Internal and upstream errors say only that.
	#[schema(example = "the sale is paid")]
	error: String,
}

/// An API error. Internal and upstream errors are logged in full and shown
/// to the caller with a generic message.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
	#[error("{0}")]
	BadRequest(String),
	#[error("the token is missing, not valid, or revoked")]
	Unauthorized,
	#[error("{0}")]
	NotFound(String),
	#[error("{0}")]
	Conflict(String),
	#[error("{0}")]
	TooManyRequests(String),
	#[error("an upstream service failed")]
	Upstream(anyhow::Error),
	#[error("internal error")]
	Internal(#[from] anyhow::Error),
}

pub type ApiResult<T> = Result<T, ApiError>;

pub fn bad_request(msg: impl Into<String>) -> ApiError {
	ApiError::BadRequest(msg.into())
}

pub fn not_found(msg: impl Into<String>) -> ApiError {
	ApiError::NotFound(msg.into())
}

pub fn conflict(msg: impl Into<String>) -> ApiError {
	ApiError::Conflict(msg.into())
}

impl From<tokio_postgres::Error> for ApiError {
	fn from(e: tokio_postgres::Error) -> Self {
		ApiError::Internal(e.into())
	}
}

impl From<bb8::RunError<tokio_postgres::Error>> for ApiError {
	fn from(e: bb8::RunError<tokio_postgres::Error>) -> Self {
		ApiError::Internal(anyhow::anyhow!("database pool: {}", e))
	}
}

impl ApiError {
	/// The status, after logging an internal or upstream error in full.
	fn status(&self) -> StatusCode {
		match self {
			ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
			ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
			ApiError::NotFound(_) => StatusCode::NOT_FOUND,
			ApiError::Conflict(_) => StatusCode::CONFLICT,
			ApiError::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
			ApiError::Upstream(e) => {
				tracing::warn!("upstream error: {:#}", e);
				StatusCode::BAD_GATEWAY
			},
			ApiError::Internal(e) => {
				tracing::error!("internal error: {:#}", e);
				StatusCode::INTERNAL_SERVER_ERROR
			},
		}
	}
}

impl IntoResponse for ApiError {
	fn into_response(self) -> Response {
		(self.status(), Json(json!({ "error": self.to_string() }))).into_response()
	}
}

/// The body of an LNURL error, as LUD-06 has it, so the payer's wallet
/// shows the reason.
#[derive(utoipa::ToSchema)]
#[allow(unused)]
pub struct LnurlErrorBody {
	#[schema(example = "ERROR")]
	status: String,
	#[schema(example = "the amount is out of range")]
	reason: String,
}

/// An [ApiError] of an LNURL endpoint, with a LUD-06 body.
#[derive(Debug)]
pub struct LnurlError(pub ApiError);

impl From<ApiError> for LnurlError {
	fn from(e: ApiError) -> Self {
		LnurlError(e)
	}
}

impl IntoResponse for LnurlError {
	fn into_response(self) -> Response {
		let status = self.0.status();
		(status, Json(json!({ "status": "ERROR", "reason": self.0.to_string() }))).into_response()
	}
}
