# Changelog

All notable changes to this project will be documented in this file.

## [0.2.0] - 2025-08-10

### 💥 Breaking Changes
- **CLI Flag Change**: Short flag for `--host` changed from `-h` to `-H` to avoid conflict with standard `--help` flag
  - **Before**: `gemini-proxy -h 0.0.0.0`
  - **After**: `gemini-proxy -H 0.0.0.0`
  - **Reason**: Eliminates confusion with help flag and follows CLI conventions
  - **Migration**: Update any scripts or deployment configurations using the old `-h` flag

### 🧪 Testing & Quality Improvements
- **MAJOR**: Expanded test coverage from 42 to 227 tests (+440% increase)
- Added comprehensive test modules:
  - `tests/main_tests.rs` (6 tests) - core application functionality
  - `tests/config_module_tests.rs` (6 tests) - configuration management
  - `tests/lib_module_tests.rs` (5 tests) - public API testing
  - `tests/simple_circuit_breaker_tests.rs` (7 tests) - circuit breaker functionality
  - `tests/key_manager_simple_tests.rs` (4 tests) - key management operations
  - `tests/error_module_tests.rs` (21 tests) - comprehensive error handling
- Achieved ~95% code coverage across all modules
- All tests passing with comprehensive edge case coverage

### 📚 Documentation Updates
- Updated README.md with accurate test count (227 tests)
- Added links to new documentation: PROJECT_STATUS_REPORT.md, DEVELOPMENT_ROADMAP.md, TEST_COVERAGE_REPORT.md
- Created comprehensive PROJECT_STATUS_REPORT.md with detailed project status analysis
- Created DEVELOPMENT_ROADMAP.md with clear roadmap for future releases (v0.3.0-v0.5.0)
- Created TEST_COVERAGE_REPORT.md with detailed test coverage analysis (~95% coverage)
- Updated badges and metrics throughout documentation
- Added breaking change documentation for CLI flag modification

### 🔧 Code Quality & Performance
- **Parallel HTTP Client Creation**: Concurrent initialization using `tokio::task::JoinSet` for faster startup
- **Rate Limiting with Retry-After**: Intelligent 429 handling with `Action::WaitFor(Duration)` support
- **Enhanced Error Handling**: 21 comprehensive error scenarios with proper HTTP status codes
- **Test Performance Optimization**: Adjusted test data sizes for CI stability while maintaining coverage
- **Import Path Corrections**: Fixed tokenizer import paths for better module organization
- **Code Cleanup**: Removed unused imports and improved code quality
- Enhanced type safety and validation throughout the codebase
- Better test isolation and independence with comprehensive edge case coverage

## [Unreleased] - 2025-08-06

- Unify default port to 4806 across docs and configs (README, QUICKSTART, MONITORING, docs/openapi.yaml, docker-compose, k8s).
- Add UAT target (make uat) with non-interactive health verification on 4806.
- Fix docs to pass audit (README/MONITORING: healthcheck, troubleshooting, busybox note, port override docs).
- Ensure cargo fmt/clippy compliance; tests green.
- Dockerfile: distroless fixes (no RUN in runtime stage), healthcheck stability, permissions for runtime-cache.

## Previous Versions

This changelog was started with the security improvements update. For earlier changes, see the git commit history.