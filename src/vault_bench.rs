//! Opt-in, dependency-free benchmarks against disposable synthetic vaults.
//! Run: cargo test --release vault::benchmarks::synthetic_vault -- --ignored --nocapture

use std::future::Future;
use std::hint::black_box;
use std::time::Instant;

use tempfile::TempDir;

use super::Vault;

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
        "{name:24} median={:.3}ms min={:.3}ms max={:.3}ms",
        samples[4], samples[0], samples[8]
    );
}

async fn benchmark_notes(note_count: usize) {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join(".obsidian")).unwrap();
    let body =
        "Plain text about Rust and MCP with Unicode café 日本語. Some #rust and #dev/backend.\n"
            .repeat(32);
    for folder in 0..20 {
        let directory = temp.path().join(format!("folder-{folder:02}"));
        std::fs::create_dir(&directory).unwrap();
        for note in 0..note_count / 20 {
            std::fs::write(
                directory.join(format!("note-{note:03}.md")),
                format!("---\ntags: [rust, mcp]\n---\n{body}See [[Target]].\n"),
            )
            .unwrap();
        }
    }
    let vault = Vault::new("benchmark".into(), temp.path().to_path_buf()).unwrap();
    println!("{note_count} notes, 20 folders, 36 lines/note; 9 samples, warmed caches");
    measure("compact list / 101", || async {
        let result = vault.list_note_paths(None, 101).await.unwrap();
        assert_eq!(result.len(), 101);
        result
    })
    .await;
    measure("detailed list / 101", || async {
        let result = vault.list_notes(None, 101).await.unwrap();
        assert_eq!(result.len(), 101);
        result
    })
    .await;
    measure("path search / 21", || async {
        let result = vault.search_notes(None, "note", 21, true, 0).await.unwrap();
        assert_eq!(result.len(), 21);
        result
    })
    .await;
    measure("content search / 21", || async {
        let result = vault
            .search_notes(None, "rust", 21, false, 0)
            .await
            .unwrap();
        assert_eq!(result.len(), 21);
        result
    })
    .await;
    measure("content search / miss", || async {
        let result = vault
            .search_notes(None, "absent-needle", 21, false, 0)
            .await
            .unwrap();
        assert!(result.is_empty());
        result
    })
    .await;
    measure("backlinks / 51", || async {
        let result = vault.get_backlinks("Target", 51, 0).await.unwrap();
        assert_eq!(result.len(), 51);
        result
    })
    .await;
    measure("backlinks / miss", || async {
        let result = vault.get_backlinks("Absent", 51, 0).await.unwrap();
        assert!(result.is_empty());
        result
    })
    .await;
    measure("list tags", || async {
        let result = vault.list_tags().await.unwrap();
        assert_eq!(result.len(), 3);
        assert!(result.iter().all(|tag| tag.count == note_count));
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
