//! Routes, body bounds, security headers and the embedded browser assets.
//! The project pages reuse the local dashboard's modules unchanged, pointed at
//! the hosted view API; cloud-only pages (sign-in, device approval, workspaces
//! and project settings) are small extra modules.
use crate::{api::Shared, cloud::error::ApiError};
use axum::{
    extract::Path,
    http::header,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use eplyx_engine::{cloud::contract::is_project_id, dashboard::assets as dashboard_assets};

macro_rules! frontend {
    ($path:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/", $path))
    };
}

const CLOUD_INDEX: &str = frontend!("cloud/index.html");
const CLOUD_ASSETS: &[(&str, &str, &str)] = &[
    (
        "parameter.js",
        "text/javascript; charset=utf-8",
        frontend!("cloud/parameter.js"),
    ),
    (
        "parameter-model.js",
        "text/javascript; charset=utf-8",
        frontend!("cloud/parameter-model.js"),
    ),
    (
        "interaction.js",
        "text/javascript; charset=utf-8",
        frontend!("cloud/interaction.js"),
    ),
    (
        "migration-order.js",
        "text/javascript; charset=utf-8",
        frontend!("cloud/migration-order.js"),
    ),
    (
        "proposal.js",
        "text/javascript; charset=utf-8",
        frontend!("cloud/proposal.js"),
    ),
    (
        "analysis.js",
        "text/javascript; charset=utf-8",
        frontend!("cloud/analysis.js"),
    ),
    (
        "migration.js",
        "text/javascript; charset=utf-8",
        frontend!("cloud/migration.js"),
    ),
    (
        "migration-model.js",
        "text/javascript; charset=utf-8",
        frontend!("cloud/migration-model.js"),
    ),
    (
        "lifecycle.js",
        "text/javascript; charset=utf-8",
        frontend!("cloud/lifecycle.js"),
    ),
    (
        "lifecycle-model.js",
        "text/javascript; charset=utf-8",
        frontend!("cloud/lifecycle-model.js"),
    ),
    (
        "cloud.js",
        "text/javascript; charset=utf-8",
        frontend!("cloud/cloud.js"),
    ),
    (
        "cloud.css",
        "text/css; charset=utf-8",
        frontend!("cloud/cloud.css"),
    ),
    (
        "settings.js",
        "text/javascript; charset=utf-8",
        frontend!("cloud/settings.js"),
    ),
];

pub fn router() -> Router<Shared> {
    Router::new()
        .route("/assets/{name}", get(asset))
        .route("/assets/main/{name}", get(main_asset))
        .route("/p/{project}", get(project_shell))
        .route("/p/{project}/", get(project_shell))
        .route("/p/{project}/{*rest}", get(project_shell_nested))
        .route("/demo", get(demo_shell))
        .route("/demo/{*rest}", get(demo_shell))
        .route("/", get(cloud_shell))
        .route("/login", get(cloud_shell))
        .route("/signup", get(cloud_shell))
        .route("/device", get(cloud_shell))
        .route("/workspaces", get(cloud_shell))
        .route("/workspaces/{*rest}", get(cloud_shell))
}

async fn asset(Path(name): Path<String>) -> Response {
    let found = CLOUD_ASSETS
        .iter()
        .find(|(asset, ..)| *asset == name)
        .map(|(_, kind, body)| (*kind, body.as_bytes()))
        .or_else(|| dashboard_assets::get(&name));
    match found {
        Some((kind, body)) => ([(header::CONTENT_TYPE, kind)], body).into_response(),
        None => ApiError::not_found("unknown asset").into_response(),
    }
}

/// The local dashboard shell, pointed at a hosted project's view API.
fn dashboard_shell(base: &str, api: &str, project: &str, demo: bool) -> Html<String> {
    let attributes = format!(
        r#"<html lang="en" data-mode="overview" data-base="{base}" data-api="{api}" data-cloud="1" data-project="{project}"{}>"#,
        if demo { r#" data-demo="1""# } else { "" }
    );
    Html(
        dashboard_assets::INDEX
            .replacen(r#"<html lang="en" data-mode="overview">"#, &attributes, 1)
            .replacen("Eplyx — Local dashboard", "Eplyx — Cloud workspace", 1)
            .replacen(
                r#"<link rel="stylesheet" href="/assets/dashboard.css" />"#,
                r#"<link rel="stylesheet" href="/assets/dashboard.css" /><link rel="stylesheet" href="/assets/cloud.css" />"#,
                1,
            ),
    )
}

async fn project_shell(Path(project): Path<String>) -> Response {
    if !is_project_id(&project) {
        return cloud_shell().await.into_response();
    }
    dashboard_shell(
        &format!("/p/{project}"),
        &format!("/v1/projects/{project}/view"),
        &project,
        false,
    )
    .into_response()
}

async fn project_shell_nested(Path((project, _)): Path<(String, String)>) -> Response {
    project_shell(Path(project)).await
}

async fn demo_shell() -> Html<String> {
    dashboard_shell("/demo", "/v1/demo/view", "", true)
}

async fn cloud_shell() -> Html<&'static str> {
    Html(CLOUD_INDEX)
}

async fn main_asset(Path(name): Path<String>) -> Response {
    let (kind, body) = match name.as_str() {
        "report.js" => ("text/javascript; charset=utf-8", frontend!("src/report.js")),
        "demo.js" => ("text/javascript; charset=utf-8", frontend!("src/demo.js")),
        "session.js" => (
            "text/javascript; charset=utf-8",
            frontend!("src/session.js"),
        ),
        "change.js" => ("text/javascript; charset=utf-8", frontend!("src/change.js")),
        "analysis.js" => (
            "text/javascript; charset=utf-8",
            frontend!("src/analysis.js"),
        ),
        "governance.js" => (
            "text/javascript; charset=utf-8",
            frontend!("src/governance.js"),
        ),
        "shell.js" => ("text/javascript; charset=utf-8", frontend!("src/shell.js")),
        "mode.js" => ("text/javascript; charset=utf-8", frontend!("src/mode.js")),
        "brand.js" => ("text/javascript; charset=utf-8", frontend!("src/brand.js")),
        "styles.css" => {
            // Scope MAIN's existing report styles to the embedded report. Keep
            // the dashboard tokens and bundled fonts; never request Google fonts.
            let css = frontend!("src/styles.css")
                .lines()
                .filter(|line| !line.starts_with("@import"))
                .collect::<Vec<_>>()
                .join("\n");
            let css = format!(
                "@scope (.hosted-upgrade-report) {{ {} }}",
                css.replace(":root", ":scope")
            );
            return ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], css).into_response();
        }
        _ => return ApiError::not_found("unknown asset").into_response(),
    };
    ([(header::CONTENT_TYPE, kind)], body).into_response()
}
