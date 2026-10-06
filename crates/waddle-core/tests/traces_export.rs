//! Exports a folder of saved tasks the way Settings does, to check the training
//! files on real data without the app:
//!
//!   WADDLE_TRACES=<traces folder> WADDLE_EXPORT=<out folder> \
//!     cargo test -p waddle-core --test traces_export -- --ignored --nocapture

use waddle_core::traces::{ExportOptions, TraceStore};

#[test]
#[ignore = "needs WADDLE_TRACES (a folder of saved tasks) and WADDLE_EXPORT"]
fn export_a_folder_of_saved_tasks() {
    let traces = std::env::var("WADDLE_TRACES").expect("WADDLE_TRACES");
    let out = std::env::var("WADDLE_EXPORT").expect("WADDLE_EXPORT");
    let store = TraceStore::new(traces.into());
    let got = store.export(std::path::Path::new(&out), ExportOptions { include_unrated: true, scrub: true }).unwrap();
    println!(
        "{} tasks -> {} examples ({} held back for false claims), {} labelled replies",
        store.list().len(),
        got.examples,
        got.held_back,
        got.labelled
    );
}
