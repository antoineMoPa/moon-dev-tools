use std::fs;

use super::*;
use crate::moontasks::store::{ColumnEnd, ColumnId};

struct Kept {
    repo_path: std::path::PathBuf,
    task_id: String,
    fragment_path: std::path::PathBuf,
}

/// A repo with one task, and a fragment written in a Codex thread's folder outside it.
fn repo_with_task_and_fragment() -> Kept {
    let root = std::env::temp_dir().join(format!("moon-kept-{}", store::new_uuid()));
    let repo_path = root.join("repo");
    fs::create_dir_all(&repo_path).unwrap();
    let repo_path = fs::canonicalize(repo_path).unwrap();
    let task_id = store::create_task(
        &repo_path,
        "chart the revenue",
        &ColumnId::new("todo"),
        ColumnEnd::Top,
    )
    .unwrap();
    let thread_dir = root.join("codex/visualizations/2026/09/16/thread");
    fs::create_dir_all(&thread_dir).unwrap();
    let fragment_path = thread_dir.join("monthly-revenue.html");
    fs::write(&fragment_path, "<div id=\"widget\">first</div>").unwrap();
    Kept {
        repo_path,
        task_id,
        fragment_path,
    }
}

fn announced(kept: &Kept) -> VisualizationView {
    VisualizationView {
        terminal_id: "terminal-1".to_string(),
        fragment_path: kept.fragment_path.display().to_string(),
        announced: 1,
        modified_unix_ms: 0,
    }
}

fn visualizations_on(kept: &Kept) -> Vec<TaskResource> {
    store::read_task(&kept.repo_path, &kept.task_id)
        .unwrap()
        .resources
        .into_iter()
        .filter(|resource| resource.kind == TaskResourceKind::Visualization)
        .collect()
}

#[test]
fn a_visualization_is_copied_into_the_task_and_listed_on_it_once() {
    let kept = repo_with_task_and_fragment();

    let view = keep_in_task_folder(&kept.repo_path, &kept.task_id, announced(&kept)).unwrap();
    keep_in_task_folder(&kept.repo_path, &kept.task_id, announced(&kept)).unwrap();

    let copy_path = store::tasks_root(&kept.repo_path)
        .join(&kept.task_id)
        .join("visualizations/monthly-revenue.html");
    assert_eq!(view.fragment_path, copy_path.display().to_string());
    assert_eq!(
        fs::read_to_string(&copy_path).unwrap(),
        "<div id=\"widget\">first</div>"
    );
    let resources = visualizations_on(&kept);
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].name.as_deref(), Some("monthly revenue"));
    assert_eq!(
        resources[0].file_path.as_deref(),
        Some(
            format!(
                ".moontasks/{}/visualizations/monthly-revenue.html",
                kept.task_id
            )
            .as_str()
        )
    );
    assert_eq!(resources[0].terminal_id, None);
    assert!(is_task_copy(&kept.repo_path, &copy_path));
}

#[test]
fn a_rewritten_fragment_is_copied_again_and_one_taken_off_comes_back() {
    let kept = repo_with_task_and_fragment();
    keep_in_task_folder(&kept.repo_path, &kept.task_id, announced(&kept)).unwrap();
    let mut metadata = store::read_task(&kept.repo_path, &kept.task_id).unwrap();
    metadata.resources.clear();
    store::write_task(&kept.repo_path, &kept.task_id, &metadata).unwrap();

    // Later than the copy by more than any clock's resolution.
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(&kept.fragment_path, "<div id=\"widget\">second</div>").unwrap();
    let view = keep_in_task_folder(&kept.repo_path, &kept.task_id, announced(&kept)).unwrap();

    assert_eq!(
        fs::read_to_string(&view.fragment_path).unwrap(),
        "<div id=\"widget\">second</div>"
    );
    assert_eq!(visualizations_on(&kept).len(), 1);
}

#[test]
fn only_an_html_file_straight_in_a_tasks_visualizations_folder_is_a_copy() {
    let kept = repo_with_task_and_fragment();
    let task_dir = store::tasks_root(&kept.repo_path).join(&kept.task_id);
    fs::create_dir_all(task_dir.join("visualizations/deeper")).unwrap();
    for (relative, content) in [
        ("notes.html", "not kept"),
        ("visualizations/chart.txt", "not html"),
        ("visualizations/deeper/chart.html", "too deep"),
    ] {
        fs::write(task_dir.join(relative), content).unwrap();
        assert!(
            !is_task_copy(&kept.repo_path, &task_dir.join(relative)),
            "{relative}"
        );
    }
    assert!(!is_task_copy(&kept.repo_path, &kept.fragment_path));
}
