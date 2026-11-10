//! Opt-in, dependency-free benchmarks against disposable synthetic vaults.
//! Run: cargo test --release vault::benchmarks::synthetic_vault -- --ignored --nocapture
//!
//! Requests are issued the way the MCP tools issue them: a page asks the vault
//! for `offset + limit + 1` results, and a complete read follows the offsets.

use std::fmt::Write as _;
use std::future::Future;
use std::hint::black_box;
use std::time::Instant;

use tempfile::TempDir;

use super::Vault;

pub(crate) const FOLDERS: usize = 20;
pub(crate) const RARE_EVERY: usize = 100; // notes containing the accented rare phrase
pub(crate) const LINK_EVERY: usize = 20; // notes linking to [[Target]]
pub(crate) const TOPICS: usize = 50; // distinct inline tags

async fn measure<T, F: Future<Output = T>>(name: &str, mut operation: impl FnMut() -> F) {
    black_box(operation().await); // Warm filesystem caches; not a cold-disk benchmark.
    let mut samples = Vec::new();
    for _ in 0..9 {
        let start = Instant::now();
        black_box(operation().await);
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{name:42} median={:9.3}ms min={:9.3}ms max={:9.3}ms",
        samples[4], samples[0], samples[8]
    );
}

// Deterministic notes of 12 to 51 lines. A third of the lines carry accents, as
// prose in many languages does, so searches exercise the non-ASCII path.
fn note_body(index: usize) -> String {
    let mut body = format!("---\ntags: [area-{}]\n---\n# Note {index}\n", index % 7);
    for line in 0..8 + index % 40 {
        match line % 3 {
            0 => writeln!(
                body,
                "Reunión del proyecto: decisión {line} sobre el diseño."
            ),
            1 => writeln!(
                body,
                "Project notes, line {line}: plain ASCII text about the plan."
            ),
            _ => writeln!(body, "- [ ] task {line} #topic-{}", index % TOPICS),
        }
        .unwrap();
    }
    if index.is_multiple_of(RARE_EVERY) {
        body.push_str("Una canción rara aparece aquí.\n");
    }
    if index.is_multiple_of(LINK_EVERY) {
        body.push_str("See [[Target]] for context.\n");
    }
    body
}

/// A disposable vault of generated notes; returns it with its total line count.
pub(crate) fn build_vault(note_count: usize) -> (TempDir, usize) {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join(".obsidian")).unwrap();
    let mut lines = 0;
    for folder in 0..FOLDERS {
        let directory = temp.path().join(format!("folder-{folder:02}"));
        std::fs::create_dir(&directory).unwrap();
        for note in 0..note_count / FOLDERS {
            let body = note_body(folder * (note_count / FOLDERS) + note);
            lines += body.lines().count();
            std::fs::write(directory.join(format!("note-{note:05}.md")), body).unwrap();
        }
    }
    (temp, lines)
}

async fn benchmark_notes(note_count: usize) {
    let (temp, lines) = build_vault(note_count);
    let vault = &Vault::new("benchmark".into(), temp.path().to_path_buf()).unwrap();
    let rare = note_count / RARE_EVERY;
    let links = note_count / LINK_EVERY;
    let middle = note_count / 2;
    println!("{note_count} notes, {FOLDERS} folders, {lines} lines; 9 samples, warmed caches");

    // One page, at the default and at the maximum page size.
    for (name, limit) in [("list 100 notes", 100), ("list 500 notes (max page)", 500)] {
        measure(name, || async move {
            let result = vault.list_note_paths(None, limit + 1, None).await.unwrap();
            assert_eq!(result.len(), limit + 1);
            result
        })
        .await;
    }
    for (name, limit) in [
        ("search paths, 20 matches", 20),
        ("search paths, 200 matches (max page)", 200),
    ] {
        measure(name, || async move {
            let result = vault
                .search_notes(None, "note", limit + 1, true, 0)
                .await
                .unwrap();
            assert_eq!(result.len(), limit + 1);
            result
        })
        .await;
    }
    for (name, limit) in [
        ("search content, 20 matches", 20),
        ("search content, 200 matches (max page)", 200),
    ] {
        measure(name, || async move {
            let result = vault
                .search_notes(None, "PROJECT", limit + 1, false, 0)
                .await
                .unwrap();
            assert_eq!(result.len(), limit + 1);
            result
        })
        .await;
    }
    measure("backlinks, 50 matches", || async move {
        let result = vault.get_backlinks("Target", 51, 0).await.unwrap();
        assert_eq!(result.len(), 51);
        result
    })
    .await;

    // A page from the middle of the vault: earlier results are skipped, not kept.
    measure("list 100 notes from the middle", || async move {
        let result = vault
            .list_note_paths(None, middle + 101, None)
            .await
            .unwrap();
        assert_eq!(result.len(), middle + 101);
        result
    })
    .await;
    measure("search content, 200 from match 10000", || async move {
        let result = vault
            .search_notes(None, "project", 201, false, 10_000)
            .await
            .unwrap();
        assert_eq!(result.len(), 201);
        result
    })
    .await;

    // Complete reads: follow the offsets until nothing is left.
    measure("list every note, 500 per page", || async move {
        let mut total = 0;
        for offset in (0..note_count).step_by(500) {
            let page = vault
                .list_note_paths(None, offset + 501, None)
                .await
                .unwrap();
            total += page.len().saturating_sub(offset).min(500);
        }
        assert_eq!(total, note_count);
        total
    })
    .await;
    measure("every backlink, 200 per page", || async move {
        let mut total = 0;
        for offset in (0..links).step_by(200) {
            let page = vault.get_backlinks("Target", 201, offset).await.unwrap();
            total += page.len().min(200);
        }
        assert_eq!(total, links);
        total
    })
    .await;
    measure("every match of a rare accented phrase", || async move {
        // Typed without accents; the notes spell it "canción rara".
        let result = vault
            .search_notes(None, "cancion rara", 201, false, 0)
            .await
            .unwrap();
        assert_eq!(result.len(), rare.min(201));
        result
    })
    .await;
    measure("every note with a tag, 500 per page", || async move {
        let expected = note_count / TOPICS;
        let mut total = 0;
        for offset in (0..expected).step_by(500) {
            let page = vault
                .list_note_paths(None, offset + 501, Some("topic-7"))
                .await
                .unwrap();
            total += page.len().saturating_sub(offset).min(500);
        }
        assert_eq!(total, expected);
        total
    })
    .await;

    // Whole-vault scans.
    measure("newest 500 notes with dates", || async move {
        let result = vault.list_notes(None, 501, None).await.unwrap();
        assert_eq!(result.len(), 501);
        result
    })
    .await;
    measure("count every tag", || async move {
        let result = vault.list_tags().await.unwrap();
        assert_eq!(result.len(), TOPICS + 7);
        result
    })
    .await;
    measure("backlinks, none found", || async move {
        let result = vault.get_backlinks("Absent", 51, 0).await.unwrap();
        assert!(result.is_empty());
        result
    })
    .await;
    measure("search content, none found", || async move {
        let result = vault
            .search_notes(None, "absent-needle", 21, false, 0)
            .await
            .unwrap();
        assert!(result.is_empty());
        result
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "opt-in performance measurement; creates only temporary synthetic vaults"]
async fn synthetic_vault() {
    benchmark_notes(2_000).await;
    benchmark_notes(20_000).await;

    let large = TempDir::new().unwrap();
    std::fs::create_dir(large.path().join(".obsidian")).unwrap();
    std::fs::write(
        large.path().join("large.md"),
        "A long line about Rust and Unicode café 日本語.\n".repeat(100_000),
    )
    .unwrap();
    let vault = Vault::new("large".into(), large.path().to_path_buf()).unwrap();
    println!("Large note: 100000 lines; 9 samples, warmed caches");
    measure("read 200 lines", || async {
        let result = vault
            .read_note_range("large.md", Some(1), Some(200))
            .await
            .unwrap();
        assert_eq!((result.1, result.2), (100_000, 200));
        result
    })
    .await;
    measure("read 500 lines from line 90000", || async {
        let result = vault
            .read_note_range("large.md", Some(90_000), Some(500))
            .await
            .unwrap();
        assert_eq!((result.1, result.2), (100_000, 500));
        result
    })
    .await;
    measure("outline", || async {
        let result = vault.get_note_outline("large.md").await.unwrap();
        assert!(result.0.is_empty());
        assert_eq!(result.1, 100_000);
        result
    })
    .await;
    measure("large-note tags", || async {
        let result = vault.list_tags().await.unwrap();
        assert!(result.is_empty());
        result
    })
    .await;
}
