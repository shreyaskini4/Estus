fn main() {
    #[cfg(windows)]
    {
        use embed_manifest::{embed_manifest, new_manifest, manifest::ExecutionLevel};
        let level = if std::env::var("PROFILE").unwrap_or_default() == "release" {
            ExecutionLevel::RequireAdministrator
        } else {
            ExecutionLevel::AsInvoker
        };
        embed_manifest(
            new_manifest("Estus")
                .requested_execution_level(level)
        ).expect("unable to embed manifest file");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
