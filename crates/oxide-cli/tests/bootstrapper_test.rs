//! Phase 5: drives the real `oxide` binary to scaffold applications and generate code.
//!
//! The fast tests check the files. `generated_apps_pass_their_own_tests` (ignored by default; run
//! with `just e2e`) compiles each generated app and runs its own `cargo test`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_oxide");

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh, empty directory under the system temp dir.
fn sandbox() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "oxide-cli-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn oxide(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn succeeds(
    Output {
        status,
        stdout,
        stderr,
    }: Output,
) -> String {
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    assert!(
        status.success(),
        "oxide failed\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    stdout
}

fn fails(Output { status, stderr, .. }: Output) -> String {
    assert!(!status.success(), "oxide unexpectedly succeeded");
    String::from_utf8_lossy(&stderr).into_owned()
}

/// `oxide new <name> -n --oxide-path <workspace>/crates/oxide <extra..>` in `dir`.
fn new_app(dir: &Path, name: &str, extra: &[&str]) -> PathBuf {
    let oxide_path = workspace().join("crates/oxide");
    let mut args = vec![
        "new",
        name,
        "-n",
        "--oxide-path",
        oxide_path.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    succeeds(oxide(dir, &args));
    dir.join(name)
}

fn read(app: &Path, path: &str) -> String {
    fs::read_to_string(app.join(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn assert_files(app: &Path, paths: &[&str]) {
    for path in paths {
        assert!(app.join(path).is_file(), "missing {path}");
    }
}

#[test]
fn new_scaffolds_the_api_blog_application() {
    let dir = sandbox();
    let app = new_app(
        &dir,
        "sample-app",
        &[
            "--preset",
            "api",
            "--database",
            "sqlite",
            "--starter",
            "blog",
        ],
    );
    assert_files(
        &app,
        &[
            "Cargo.toml",
            ".env.example",
            ".gitignore",
            "README.md",
            "build.rs",
            "database/migrations/0001_create_posts_table.sql",
            "src/main.rs",
            "src/lib.rs",
            "src/context.rs",
            "src/config.rs",
            "config/app.toml",
            "config/database.toml",
            "src/models/post.rs",
            "src/http/controllers/post_controller.rs",
            "src/http/requests/store_post_request.rs",
            "src/routes/api.rs",
            "tests/post_feature_test.rs",
            ".claude/skills/oxide-eloquent-rust/SKILL.md",
            ".cursor/rules/oxide-eloquent-rust.mdc",
        ],
    );

    let manifest = read(&app, "Cargo.toml");
    assert!(manifest.contains("name = \"sample-app\""));
    assert!(manifest.contains("oxide = { path = "));
    assert!(manifest.contains("database = \"sqlite\""));
    assert!(read(&app, "tests/post_feature_test.rs").contains("use sample_app::"));
    assert!(read(&app, "src/routes/api.rs").contains(r#"resource("/posts", PostController);"#));
    assert!(read(&app, ".env.example").contains("DATABASE_URL=sqlite://"));
    assert!(read(&app, "config/database.toml").contains("url = \"sqlite://"));
    assert!(read(&app, "src/config.rs").contains("pub struct DatabaseConfig"));
    assert!(
        !read(&app, ".claude/skills/oxide-eloquent-rust/SKILL.md")
            .contains("Repository conventions")
    );
}

#[test]
fn new_honours_presets_databases_and_the_skill_flag() {
    let dir = sandbox();

    let blank = new_app(&dir, "blank-app", &["--starter", "blank", "--no-ai-skill"]);
    assert_files(
        &blank,
        &[
            "tests/health_test.rs",
            "src/models/mod.rs",
            "database/migrations/.gitkeep",
        ],
    );
    assert!(!blank.join("src/models/post.rs").exists());
    assert!(!blank.join(".claude").exists() && !blank.join(".cursor").exists());

    let minimal = new_app(&dir, "tiny-app", &["--preset", "minimal"]);
    assert_files(
        &minimal,
        &[
            "tests/health_test.rs",
            "src/context.rs",
            "src/config.rs",
            "config/app.toml",
        ],
    );
    assert!(!minimal.join("config/database.toml").exists());
    assert!(!read(&minimal, "src/config.rs").contains("DatabaseConfig"));
    assert!(!minimal.join("database").exists() && !minimal.join("src/models").exists());
    let manifest = read(&minimal, "Cargo.toml");
    assert!(manifest.contains("database = \"none\"") && !manifest.contains("sqlx"));

    let postgres = new_app(&dir, "pg-app", &["--database", "postgres"]);
    assert!(read(&postgres, "Cargo.toml").contains(r#"features = ["postgres"]"#));
    assert!(
        read(&postgres, "database/migrations/0001_create_posts_table.sql").contains("BIGSERIAL")
    );
    assert!(read(&postgres, ".env.example").contains("DATABASE_URL=postgres://"));
}

#[test]
fn make_commands_create_and_register_files() {
    let dir = sandbox();
    let app = new_app(&dir, "sample-app", &[]);

    let out = succeeds(oxide(&app, &["make:model", "Comment", "-m"]));
    assert!(out.contains("Model [src/models/comment.rs] created successfully."));
    assert!(out.contains("database/migrations/0002_create_comments_table.sql"));
    succeeds(oxide(&app, &["make:controller", "Comment", "--resource"]));
    succeeds(oxide(&app, &["make:request", "StoreCommentRequest"]));

    assert_files(
        &app,
        &[
            "src/models/comment.rs",
            "database/migrations/0002_create_comments_table.sql",
            "src/http/controllers/comment_controller.rs",
            "src/http/requests/store_comment_request.rs",
        ],
    );
    let models = read(&app, "src/models/mod.rs");
    assert!(models.contains("pub mod comment;") && models.contains("pub use comment::Comment;"));
    assert!(read(&app, "src/http/controllers/mod.rs").contains("pub mod comment_controller;"));
    assert!(read(&app, "src/http/requests/mod.rs").contains("pub mod store_comment_request;"));
    let controller = read(&app, "src/http/controllers/comment_controller.rs");
    assert!(controller.contains("pub struct CommentController;"));
    for action in ["index", "store", "show", "update", "destroy"] {
        assert!(
            controller.contains(&format!("pub async fn {action}()")),
            "missing {action}"
        );
    }
    assert!(read(&app, "src/models/comment.rs").contains(r#"#[model(table = "comments")]"#));
    assert!(
        read(&app, "database/migrations/0002_create_comments_table.sql")
            .contains(r#"CREATE TABLE "comments""#)
    );

    // Existing files are kept unless forced, and registration never duplicates.
    let error = fails(oxide(&app, &["make:model", "Comment"]));
    assert!(error.contains("already exists; use --force"));
    succeeds(oxide(&app, &["make:model", "Comment", "--force"]));
    assert_eq!(read(&app, "src/models/mod.rs"), models);

    // Forcing `-m` again rewrites the comments migration instead of adding a second one.
    succeeds(oxide(&app, &["make:model", "Comment", "-m", "--force"]));
    let migrations: Vec<_> = fs::read_dir(app.join("database/migrations"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with("_create_comments_table.sql"))
        .collect();
    assert_eq!(migrations, ["0002_create_comments_table.sql"]);
}

#[test]
fn bad_input_fails_cleanly() {
    let dir = sandbox();
    let oxide_path = workspace().join("crates/oxide");
    let oxide_path = oxide_path.to_str().unwrap();

    assert!(fails(oxide(&dir, &["new", "My_App", "-n"])).contains("not a valid project name"));
    assert!(fails(oxide(&dir, &["new", "-n"])).contains("a project name is required"));
    assert!(
        fails(oxide(
            &dir,
            &[
                "new",
                "tiny-app",
                "-n",
                "--preset",
                "minimal",
                "--database",
                "postgres"
            ]
        ))
        .contains("the minimal preset has no database")
    );
    assert!(fails(oxide(&dir, &["make:model", "Comment"])).contains("is not an Oxide application"));

    new_app(&dir, "taken", &[]);
    assert!(
        fails(oxide(
            &dir,
            &["new", "taken", "-n", "--oxide-path", oxide_path]
        ))
        .contains("already exists and is not empty")
    );

    let minimal = new_app(&dir, "tiny-app", &["--preset", "minimal"]);
    assert!(fails(oxide(&minimal, &["make:model", "Comment"])).contains("has no database"));
    assert!(fails(oxide(&minimal, &["make:request", "bad-name"])).contains("not a valid name"));

    // Names that would generate keywords: `pub mod type;`, `struct Self`, `use async::..`.
    for name in ["Type", "Mod", "Self"] {
        let error = fails(oxide(&minimal, &["make:request", name]));
        assert!(error.contains("keyword"), "{name}: {error}");
    }
    assert!(fails(oxide(&dir, &["new", "async", "-n"])).contains("reserved"));
}

/// Runs `cargo test` in `app` with the workspace's lockfile and a shared target directory, so
/// dependencies match the workspace and compile once for every generated app.
fn cargo_test(app: &Path, env: &[(&str, &str)]) {
    fs::copy(workspace().join("Cargo.lock"), app.join("Cargo.lock")).unwrap();
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .arg("test")
        .current_dir(app)
        .env("CARGO_TARGET_DIR", workspace().join("target/e2e"))
        .env("RUSTFLAGS", "-D warnings")
        .envs(env.iter().copied())
        .status()
        .unwrap();
    assert!(status.success(), "`cargo test` failed in {}", app.display());
}

#[test]
#[ignore = "compiles generated apps; run with `just e2e`"]
fn generated_apps_pass_their_own_tests() {
    let dir = sandbox();

    let blog = new_app(&dir, "blog-app", &[]);
    succeeds(oxide(&blog, &["make:model", "Comment", "-m"]));
    succeeds(oxide(&blog, &["make:controller", "Comment", "--resource"]));
    succeeds(oxide(&blog, &["make:controller", "Dashboard"]));
    succeeds(oxide(&blog, &["make:request", "StoreCommentRequest"]));
    // `values` is an SQL keyword: the migration must quote it.
    succeeds(oxide(&blog, &["make:model", "Value", "-m"]));
    cargo_test(&blog, &[]);

    cargo_test(&new_app(&dir, "blank-app", &["--starter", "blank"]), &[]);
    cargo_test(&new_app(&dir, "tiny-app", &["--preset", "minimal"]), &[]);

    if let Ok(url) = std::env::var("OXIDE_TEST_POSTGRES_URL") {
        let postgres = new_app(&dir, "pg-app", &["--database", "postgres"]);
        succeeds(oxide(&postgres, &["make:model", "Comment", "-m"]));
        cargo_test(&postgres, &[("DATABASE_URL", url.as_str())]);
    } else {
        assert!(
            std::env::var_os("CI").is_none(),
            "OXIDE_TEST_POSTGRES_URL must be set in CI"
        );
        eprintln!("skipping the PostgreSQL app: set OXIDE_TEST_POSTGRES_URL");
    }
}
