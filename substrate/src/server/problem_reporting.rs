use std::sync::atomic::{AtomicI32, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    problem_reporting_service_server::ProblemReportingService, ClearProblemsRequest,
    ClearProblemsResponse, GetProblemsBySeverityRequest, GetProblemsRequest,
    GetProblemsResponse, ProblemDetails, ReportProblemRequest, ReportProblemResponse,
};

/// Rust-native problem/diagnostic reporting service.
/// Collects build problems, warnings, deprecations, and errors.
/// Provides structured diagnostics for IDEs and CI dashboards.
pub struct ProblemReportingServiceImpl {
    problems: DashMap<String, Vec<ProblemDetails>>, // build_id -> [ProblemDetails]
    next_problem_id: AtomicI32,
    problems_reported: AtomicI32,
}

impl Default for ProblemReportingServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl ProblemReportingServiceImpl {
    pub fn new() -> Self {
        Self {
            problems: DashMap::new(),
            next_problem_id: AtomicI32::new(1),
            problems_reported: AtomicI32::new(0),
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

#[tonic::async_trait]
impl ProblemReportingService for ProblemReportingServiceImpl {
    async fn report_problem(
        &self,
        request: Request<ReportProblemRequest>,
    ) -> Result<Response<ReportProblemResponse>, Status> {
        let req = request.into_inner();

        let mut problem = req
            .problem
            .ok_or_else(|| Status::invalid_argument("ProblemDetails is required"))?;

        // Assign an ID and timestamp if not set
        if problem.problem_id.is_empty() {
            let id = self.next_problem_id.fetch_add(1, Ordering::Relaxed);
            problem.problem_id = format!("problem-{}", id);
        }
        if problem.timestamp_ms == 0 {
            problem.timestamp_ms = Self::now_ms();
        }

        let severity = problem.severity.clone();
        let category = problem.category.clone();

        self.problems
            .entry(req.build_id.clone())
            .or_default()
            .push(problem);

        self.problems_reported.fetch_add(1, Ordering::Relaxed);

        tracing::debug!(
            build_id = %req.build_id,
            severity = %severity,
            category = %category,
            "Problem reported"
        );

        Ok(Response::new(ReportProblemResponse { accepted: true }))
    }

    async fn get_problems(
        &self,
        request: Request<GetProblemsRequest>,
    ) -> Result<Response<GetProblemsResponse>, Status> {
        let req = request.into_inner();

        let all_problems = self
            .problems
            .get(&req.build_id)
            .map(|p| p.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let total = all_problems.len() as i32;
        let warning_count = all_problems.iter().filter(|p| p.severity == "warning").count() as i32;
        let error_count = all_problems.iter().filter(|p| p.severity == "error").count() as i32;
        let deprecation_count = all_problems
            .iter()
            .filter(|p| p.severity == "deprecation")
            .count() as i32;

        Ok(Response::new(GetProblemsResponse {
            problems: all_problems,
            total,
            warning_count,
            error_count,
            deprecation_count,
        }))
    }

    async fn get_problems_by_severity(
        &self,
        request: Request<GetProblemsBySeverityRequest>,
    ) -> Result<Response<GetProblemsResponse>, Status> {
        let req = request.into_inner();

        let all_problems = self
            .problems
            .get(&req.build_id)
            .map(|p| {
                p.iter()
                    .filter(|p| p.severity == req.severity)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let total = all_problems.len() as i32;
        let warning_count = if req.severity == "warning" { total } else { 0 };
        let error_count = if req.severity == "error" { total } else { 0 };
        let deprecation_count = if req.severity == "deprecation" { total } else { 0 };

        Ok(Response::new(GetProblemsResponse {
            problems: all_problems,
            total,
            warning_count,
            error_count,
            deprecation_count,
        }))
    }

    async fn clear_problems(
        &self,
        request: Request<ClearProblemsRequest>,
    ) -> Result<Response<ClearProblemsResponse>, Status> {
        let req = request.into_inner();
        let cleared = if let Some((_, problems)) = self.problems.remove(&req.build_id) {
            problems.len() as i32
        } else {
            0
        };

        Ok(Response::new(ClearProblemsResponse { cleared }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_problem(severity: &str, category: &str, message: &str) -> ProblemDetails {
        ProblemDetails {
            problem_id: String::new(),
            severity: severity.to_string(),
            category: category.to_string(),
            message: message.to_string(),
            details: String::new(),
            file_path: String::new(),
            line_number: 0,
            column: 0,
            contextual_label: String::new(),
            documentation_url: String::new(),
            additional_data: String::new(),
            timestamp_ms: 0,
        }
    }

    #[tokio::test]
    async fn test_report_and_get_problems() {
        let svc = ProblemReportingServiceImpl::new();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-1".to_string(),
            problem: Some(make_problem("warning", "deprecated_feature", "Old API used")),
        }))
        .await
        .unwrap();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-1".to_string(),
            problem: Some(make_problem("error", "compile", "Type mismatch")),
        }))
        .await
        .unwrap();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-1".to_string(),
            problem: Some(make_problem("deprecation", "property_override", "Property X is deprecated")),
        }))
        .await
        .unwrap();

        let resp = svc
            .get_problems(Request::new(GetProblemsRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total, 3);
        assert_eq!(resp.warning_count, 1);
        assert_eq!(resp.error_count, 1);
        assert_eq!(resp.deprecation_count, 1);
    }

    #[tokio::test]
    async fn test_filter_by_severity() {
        let svc = ProblemReportingServiceImpl::new();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-2".to_string(),
            problem: Some(make_problem("warning", "lint", "Unused import")),
        }))
        .await
        .unwrap();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-2".to_string(),
            problem: Some(make_problem("warning", "lint", "Unused variable")),
        }))
        .await
        .unwrap();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-2".to_string(),
            problem: Some(make_problem("error", "compile", "Missing class")),
        }))
        .await
        .unwrap();

        let resp = svc
            .get_problems_by_severity(Request::new(GetProblemsBySeverityRequest {
                build_id: "build-2".to_string(),
                severity: "warning".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total, 2);
        assert_eq!(resp.warning_count, 2);
        assert_eq!(resp.error_count, 0);
    }

    #[tokio::test]
    async fn test_clear_problems() {
        let svc = ProblemReportingServiceImpl::new();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-3".to_string(),
            problem: Some(make_problem("info", "general", "Some info")),
        }))
        .await
        .unwrap();

        let resp = svc
            .clear_problems(Request::new(ClearProblemsRequest {
                build_id: "build-3".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.cleared, 1);

        let resp = svc
            .get_problems(Request::new(GetProblemsRequest {
                build_id: "build-3".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total, 0);
    }

    #[tokio::test]
    async fn test_problem_auto_id() {
        let svc = ProblemReportingServiceImpl::new();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-4".to_string(),
            problem: Some(make_problem("info", "test", "First")),
        }))
        .await
        .unwrap();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-4".to_string(),
            problem: Some(make_problem("info", "test", "Second")),
        }))
        .await
        .unwrap();

        let resp = svc
            .get_problems(Request::new(GetProblemsRequest {
                build_id: "build-4".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.problems[0].problem_id, "problem-1");
        assert_eq!(resp.problems[1].problem_id, "problem-2");
    }

    #[tokio::test]
    async fn test_clear_nonexistent_build() {
        let svc = ProblemReportingServiceImpl::new();

        let resp = svc
            .clear_problems(Request::new(ClearProblemsRequest {
                build_id: "nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.cleared, 0);
    }

    #[tokio::test]
    async fn test_filter_by_error_severity() {
        let svc = ProblemReportingServiceImpl::new();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-err".to_string(),
            problem: Some(make_problem("error", "compile", "Type error")),
        }))
        .await
        .unwrap();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-err".to_string(),
            problem: Some(make_problem("error", "compile", "Missing method")),
        }))
        .await
        .unwrap();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-err".to_string(),
            problem: Some(make_problem("warning", "lint", "Unused var")),
        }))
        .await
        .unwrap();

        let resp = svc
            .get_problems_by_severity(Request::new(GetProblemsBySeverityRequest {
                build_id: "build-err".to_string(),
                severity: "error".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total, 2);
        assert_eq!(resp.error_count, 2);
        assert_eq!(resp.warning_count, 0);
        assert_eq!(resp.deprecation_count, 0);
    }

    #[tokio::test]
    async fn test_multiple_builds_isolated() {
        let svc = ProblemReportingServiceImpl::new();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-x".to_string(),
            problem: Some(make_problem("error", "test", "Build X error")),
        }))
        .await
        .unwrap();

        svc.report_problem(Request::new(ReportProblemRequest {
            build_id: "build-y".to_string(),
            problem: Some(make_problem("warning", "test", "Build Y warning")),
        }))
        .await
        .unwrap();

        let resp_x = svc
            .get_problems(Request::new(GetProblemsRequest {
                build_id: "build-x".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let resp_y = svc
            .get_problems(Request::new(GetProblemsRequest {
                build_id: "build-y".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp_x.total, 1);
        assert_eq!(resp_x.error_count, 1);
        assert_eq!(resp_y.total, 1);
        assert_eq!(resp_y.warning_count, 1);
    }

    #[tokio::test]
    async fn test_empty_build() {
        let svc = ProblemReportingServiceImpl::new();

        let resp = svc
            .get_problems(Request::new(GetProblemsRequest {
                build_id: "nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total, 0);
        assert_eq!(resp.warning_count, 0);
        assert_eq!(resp.error_count, 0);
    }
}
