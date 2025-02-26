use std::{ops::Deref, sync::Arc};

use anyhow::Result;
use napi::{bindgen_prelude::External, JsFunction};
use next_api::{
    paths::ServerPath,
    route::{Endpoint, WrittenEndpoint},
};
use tracing::Instrument;
use turbo_tasks::{Completion, ReadRef, Vc};
use turbopack_binding::turbopack::core::{
    diagnostics::PlainDiagnostic, error::PrettyPrintError, issue::PlainIssue,
};

use super::utils::{
    get_diagnostics, get_issues, subscribe, NapiDiagnostic, NapiIssue, RootTask, TurbopackResult,
    VcArc,
};

#[napi(object)]
#[derive(Default)]
pub struct NapiEndpointConfig {}

#[napi(object)]
#[derive(Default)]
pub struct NapiServerPath {
    pub path: String,
    pub content_hash: String,
}

impl From<ServerPath> for NapiServerPath {
    fn from(server_path: ServerPath) -> Self {
        Self {
            path: server_path.path,
            content_hash: format!("{:x}", server_path.content_hash),
        }
    }
}

#[napi(object)]
#[derive(Default)]
pub struct NapiWrittenEndpoint {
    pub r#type: String,
    pub entry_path: Option<String>,
    pub client_paths: Vec<String>,
    pub server_paths: Vec<NapiServerPath>,
    pub config: NapiEndpointConfig,
}

impl From<WrittenEndpoint> for NapiWrittenEndpoint {
    fn from(written_endpoint: WrittenEndpoint) -> Self {
        match written_endpoint {
            WrittenEndpoint::NodeJs {
                server_entry_path,
                server_paths,
                client_paths,
            } => Self {
                r#type: "nodejs".to_string(),
                entry_path: Some(server_entry_path),
                client_paths: client_paths.into_iter().map(From::from).collect(),
                server_paths: server_paths.into_iter().map(From::from).collect(),
                ..Default::default()
            },
            WrittenEndpoint::Edge {
                server_paths,
                client_paths,
            } => Self {
                r#type: "edge".to_string(),
                client_paths: client_paths.into_iter().map(From::from).collect(),
                server_paths: server_paths.into_iter().map(From::from).collect(),
                ..Default::default()
            },
        }
    }
}

// NOTE(alexkirsz) We go through an extra layer of indirection here because of
// two factors:
// 1. rustc currently has a bug where using a dyn trait as a type argument to
//    some async functions (in this case `endpoint_write_to_disk`) can cause
//    higher-ranked lifetime errors. See https://github.com/rust-lang/rust/issues/102211
// 2. the type_complexity clippy lint.
pub struct ExternalEndpoint(pub VcArc<Vc<Box<dyn Endpoint>>>);

impl Deref for ExternalEndpoint {
    type Target = VcArc<Vc<Box<dyn Endpoint>>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[turbo_tasks::value(serialization = "none")]
struct WrittenEndpointWithIssues {
    written: ReadRef<WrittenEndpoint>,
    issues: Arc<Vec<ReadRef<PlainIssue>>>,
    diagnostics: Arc<Vec<ReadRef<PlainDiagnostic>>>,
}

#[turbo_tasks::function]
async fn get_written_endpoint_with_issues(
    endpoint: Vc<Box<dyn Endpoint>>,
) -> Result<Vc<WrittenEndpointWithIssues>> {
    let write_to_disk = endpoint.write_to_disk();
    let written = write_to_disk.strongly_consistent().await?;
    let issues = get_issues(write_to_disk).await?;
    let diagnostics = get_diagnostics(write_to_disk).await?;
    Ok(WrittenEndpointWithIssues {
        written,
        issues,
        diagnostics,
    }
    .cell())
}

#[napi]
#[tracing::instrument(skip_all)]
pub async fn endpoint_write_to_disk(
    #[napi(ts_arg_type = "{ __napiType: \"Endpoint\" }")] endpoint: External<ExternalEndpoint>,
) -> napi::Result<TurbopackResult<NapiWrittenEndpoint>> {
    let turbo_tasks = endpoint.turbo_tasks().clone();
    let endpoint = ***endpoint;
    let (written, issues, diags) = turbo_tasks
        .run_once(async move {
            let WrittenEndpointWithIssues {
                written,
                issues,
                diagnostics,
            } = &*get_written_endpoint_with_issues(endpoint)
                .strongly_consistent()
                .await?;
            Ok((written.clone(), issues.clone(), diagnostics.clone()))
        })
        .await
        .map_err(|e| napi::Error::from_reason(PrettyPrintError(&e).to_string()))?;
    Ok(TurbopackResult {
        result: NapiWrittenEndpoint::from(written.clone_value()),
        issues: issues.iter().map(|i| NapiIssue::from(&**i)).collect(),
        diagnostics: diags.iter().map(|d| NapiDiagnostic::from(d)).collect(),
    })
}

#[napi(ts_return_type = "{ __napiType: \"RootTask\" }")]
pub fn endpoint_server_changed_subscribe(
    #[napi(ts_arg_type = "{ __napiType: \"Endpoint\" }")] endpoint: External<ExternalEndpoint>,
    issues: bool,
    func: JsFunction,
) -> napi::Result<External<RootTask>> {
    let turbo_tasks = endpoint.turbo_tasks().clone();
    let endpoint = ***endpoint;
    subscribe(
        turbo_tasks,
        func,
        move || {
            async move {
                subscribe_issues_and_diags(endpoint, issues)
                    .strongly_consistent()
                    .await
            }
            .instrument(tracing::info_span!("server changes subscription"))
        },
        |ctx| {
            let EndpointIssuesAndDiags {
                changed: _,
                issues,
                diagnostics,
            } = &*ctx.value;

            Ok(vec![TurbopackResult {
                result: (),
                issues: issues.iter().map(|i| NapiIssue::from(&**i)).collect(),
                diagnostics: diagnostics
                    .iter()
                    .map(|d| NapiDiagnostic::from(d))
                    .collect(),
            }])
        },
    )
}

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
struct EndpointIssuesAndDiags {
    changed: ReadRef<Completion>,
    issues: Arc<Vec<ReadRef<PlainIssue>>>,
    diagnostics: Arc<Vec<ReadRef<PlainDiagnostic>>>,
}

impl PartialEq for EndpointIssuesAndDiags {
    fn eq(&self, other: &Self) -> bool {
        ReadRef::ptr_eq(&self.changed, &other.changed)
            && self.issues == other.issues
            && self.diagnostics == other.diagnostics
    }
}

impl Eq for EndpointIssuesAndDiags {}

#[turbo_tasks::function]
async fn subscribe_issues_and_diags(
    endpoint: Vc<Box<dyn Endpoint>>,
    should_include_issues: bool,
) -> Result<Vc<EndpointIssuesAndDiags>> {
    let changed = endpoint.server_changed();
    let changed_value = changed.strongly_consistent().await?;

    if should_include_issues {
        let issues = get_issues(changed).await?;
        let diagnostics = get_diagnostics(changed).await?;
        Ok(EndpointIssuesAndDiags {
            changed: changed_value,
            issues,
            diagnostics,
        }
        .cell())
    } else {
        Ok(EndpointIssuesAndDiags {
            changed: changed_value,
            issues: Arc::new(vec![]),
            diagnostics: Arc::new(vec![]),
        }
        .cell())
    }
}

#[napi(ts_return_type = "{ __napiType: \"RootTask\" }")]
pub fn endpoint_client_changed_subscribe(
    #[napi(ts_arg_type = "{ __napiType: \"Endpoint\" }")] endpoint: External<ExternalEndpoint>,
    func: JsFunction,
) -> napi::Result<External<RootTask>> {
    let turbo_tasks = endpoint.turbo_tasks().clone();
    let endpoint = ***endpoint;
    subscribe(
        turbo_tasks,
        func,
        move || {
            async move {
                let changed = endpoint.client_changed();
                // We don't capture issues and diagonistics here since we don't want to be
                // notified when they change
                changed.strongly_consistent().await?;
                Ok(())
            }
            .instrument(tracing::info_span!("client changes subscription"))
        },
        |_| {
            Ok(vec![TurbopackResult {
                result: (),
                issues: vec![],
                diagnostics: vec![],
            }])
        },
    )
}
