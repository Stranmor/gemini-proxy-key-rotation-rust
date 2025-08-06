# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased] - 2025-08-06

- Unify default port to 4806 across docs and configs (README, QUICKSTART, MONITORING, docs/openapi.yaml, docker-compose, k8s).
- Add UAT target (make uat) with non-interactive health verification on 4806.
- Fix docs to pass audit (README/MONITORING: healthcheck, troubleshooting, busybox note, port override docs).
- Ensure cargo fmt/clippy compliance; tests green.
- Dockerfile: distroless fixes (no RUN in runtime stage), healthcheck stability, permissions for runtime-cache.

## Previous Versions

This changelog was started with the security improvements update. For earlier changes, see the git commit history.