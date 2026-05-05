fn main() {
    // Surface GIT_COMMIT (set at build time via Dockerfile ARG/ENV) into the
    // compiled binary so env!("GIT_COMMIT") works at runtime. Falls back to
    // "unknown" for local builds where the env var isn't set.
    let commit = std::env::var("GIT_COMMIT").unwrap_or_else(|_| "unknown".to_string());
    let short: String = commit.chars().take(7).collect();
    println!("cargo:rustc-env=GIT_COMMIT={}", short);
    println!("cargo:rerun-if-env-changed=GIT_COMMIT");
}
