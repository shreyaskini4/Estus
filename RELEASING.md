# Estus Release Process

This document outlines the complete release procedure for maintainers of **Estus**. It covers pre-release verification, version bumping, tagging, automated GitHub Actions builds, and manual release dispatching.

---

## 1. Overview

Releases for Estus are automated using GitHub Actions via the workflow defined in [`.github/workflows/release.yml`](.github/workflows/release.yml).

The release pipeline can be triggered in two ways:
1. **Automated Tag Push (Standard):** Pushing any Git tag matching the version pattern `v*` (for example, `v0.1.0` or `v0.1.1`) to GitHub automatically triggers the build and release workflow.
2. **Manual Dispatch (`workflow_dispatch`):** Maintainers can trigger the release workflow on-demand directly from the GitHub Actions web interface for testing or manual builds.

---

## 2. Pre-Release Verification Checklist

Before creating a new release or tagging a commit, maintainers must verify that the codebase compiles cleanly, passes all tests, conforms to lint standards, and satisfies setup script diagnostics.

Run the following commands locally in PowerShell from the repository root:

### 1. Run Unit & Integration Tests
Ensure all unit and mock tests pass in release mode. The `ESTUS_TEST=1` environment variable enables mock/test environments without requiring physical drive hardware:
```powershell
$env:ESTUS_TEST = "1"; cargo test --release
```

### 2. Run Clippy Linter
Ensure no linter warnings or code quality violations exist across all targets (library, binaries, tests, benchmarks):
```powershell
cargo clippy --release --all-targets -- -D warnings
```

### 3. Verify Compilation
Verify that the release target compiles without issues:
```powershell
cargo check --release
```

### 4. Verify Setup and Health Check Script
Run the built-in repository setup script in verification mode to ensure all dependencies and tooling prerequisites are properly configured:
```powershell
.\setup.ps1 -Check
```

---

## 3. Release Steps

Follow these steps in order when releasing a new version:

### Step 1: Bump Version in `Cargo.toml`
Open `Cargo.toml` and update the `version` field in the `[package]` table:

```toml
[package]
name = "estus"
version = "0.1.1" # Update to the target version
edition = "2024"
```

### Step 2: Update `Cargo.lock`
Ensure `Cargo.lock` reflects the updated version number by running:
```powershell
cargo check --release
```

### Step 3: Commit the Version Bump
Stage and commit the modified `Cargo.toml` and `Cargo.lock` files:
```powershell
git commit -am "chore: release v0.1.1"
```

### Step 4: Tag the Commit
Create an annotated or standard Git tag matching the release version prefixed with `v`:
```powershell
git tag v0.1.1
```

### Step 5: Push Commit and Tag to GitHub
Push both the commit on `main` and the newly created tag to GitHub:
```powershell
git push origin main
git push origin v0.1.1
```

---

## 4. GitHub Actions Automation

Once the `v*` tag is pushed to GitHub, `.github/workflows/release.yml` automatically executes the release pipeline:

1. **Environment Setup:**
   - Runs on `windows-latest` with the MSVC toolchain (Rust stable).
2. **Compilation:**
   - Compiles the optimized release binary:
     ```powershell
     cargo build --release --locked
     ```
3. **Artifact Packaging:**
   - Packages `target\release\estus.exe` and `README.md` into a standardized archive named:
     ```text
     Estus-v<VERSION>-windows-x64.zip
     ```
     *(e.g., `Estus-v0.1.1-windows-x64.zip`)*
4. **Checksum Generation:**
   - Computes the SHA-256 hash of the archive and saves it alongside the zip file:
     ```text
     Estus-v<VERSION>-windows-x64.zip.sha256
     ```
5. **GitHub Release Creation:**
   - Automatically publishes a GitHub Release associated with the Git tag.
   - Generates release notes based on recent commits and pull requests.
   - Uploads both the ZIP distribution package and its SHA-256 checksum file as release assets.

---

## 5. Manual Workflow Dispatch

If a release build needs to be created or re-triggered without pushing a new tag:

1. Navigate to the repository on GitHub.
2. Click on the **Actions** tab in the top navigation bar.
3. In the left sidebar under *Workflows*, select **Release**.
4. Click the **Run workflow** dropdown button on the right.
5. Select the branch (usually `main`) or tag to run against.
6. Click **Run workflow** to start the job.

The workflow will compile the binary, package the zip archive and checksum, and publish/update the release accordingly.

---

## 6. Post-Release Verification

After the GitHub Actions workflow completes:
- Verify that the release appears under the repository's **Releases** page (`/releases`).
- Download the generated ZIP package (`Estus-v<VERSION>-windows-x64.zip`) and verify its SHA-256 hash against the `.sha256` file:
  ```powershell
  Get-FileHash .\Estus-v0.1.1-windows-x64.zip -Algorithm SHA256
  Get-Content .\Estus-v0.1.1-windows-x64.zip.sha256
  ```
- Test extracting and launching `estus.exe` on a clean Windows machine to verify proper startup and system tray integration.
