//! Build identity and supported artifact schemas; no runtime paths or environment.
use crate::{bundle, change, ci, local_store, migration as m};
use serde_json::{json, Value};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("EPLYX_BUILD_COMMIT");
pub const TARGET: &str = env!("EPLYX_BUILD_TARGET");
pub fn short_commit() -> &'static str {
    COMMIT.get(..12).unwrap_or(COMMIT)
}
pub fn platform() -> String {
    format!(
        "{}-{}",
        match std::env::consts::OS {
            "macos" => "darwin",
            x => x,
        },
        match std::env::consts::ARCH {
            "aarch64" => "arm64",
            x => x,
        }
    )
}
pub fn long_version() -> String {
    format!("{VERSION}\ncommit {}\ntarget {TARGET}\nengine eplyx-engine {VERSION} · ChangeSpec {} · migration input {} · {}",short_commit(),change::CHANGE_SPEC_SCHEMA,m::input::SCHEMA_VERSION,m::search::SEARCH_VERSION)
}
pub fn json() -> Value {
    json!({
        "schema_version":1,"name":"eplyx","version":VERSION,"commit":COMMIT,"target":TARGET,"platform":platform(),"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "engine":{"crate":"eplyx-engine","version":VERSION,"change_spec_schema":change::CHANGE_SPEC_SCHEMA,"ci_report_schema":ci::CI_REPORT_SCHEMA,"bundle_schema":bundle::CI_BUNDLE_SCHEMA,
            "change_specs":{"program_upgrade":[1],"token_migration":[1]},"run_metadata_schema":local_store::METADATA_VERSION,"reproduction_schema":local_store::REPRODUCTION_VERSION,
            "token_migration":{"adapter":m::adapter::ADAPTER,"adapter_version":m::adapter::ADAPTER_VERSION,"state_input_schema":m::input::SCHEMA_VERSION,"report_schema":m::report::REPORT_SCHEMA,"invariant_schema":m::invariants::INVARIANT_SCHEMA_VERSION,"fixture_recipe_schema":m::fixture::RECIPE_SCHEMA,"unsigned_plan_schema":m::unsigned::UNSIGNED_SCHEMA,"counterexample_search":m::search::SEARCH_VERSION,"token_2022_matrix":m::extensions::MATRIX_VERSION,"capture_schema":m::capture::CAPTURE_SCHEMA,"bindings_schema":m::pipeline::BINDINGS_SCHEMA,"plan_schema":m::planner::PLAN_SCHEMA,"planner":m::planner::PLANNER_VERSION,"executor":m::execute::EXECUTOR_VERSION,"rehearsal":m::rehearsal::REHEARSAL_VERSION,"stress":m::stress::STRESS_VERSION,"current":m::current::VERSION,"current_selector":m::current_select::SELECTOR_VERSION_V2,"observed_search":m::observed_search::VERSION,"authority_selector":m::authority_resolution::SELECTOR_VERSION,"authority_resolver":m::authority_resolution::RESOLVER_VERSION}
        }
    })
}
