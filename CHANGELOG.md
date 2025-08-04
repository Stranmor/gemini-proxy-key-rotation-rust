# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased] - 2025-01-XX

### Security
- **CRITICAL**: Fixed API key leakage in logs - keys are now properly masked
- Added constant-time comparison for authentication tokens to prevent timing attacks
- Added request size limits (10MB) to prevent DoS attacks through large requests
- Added CSRF token size validation (128 character limit)
- Improved Redis KEYS command safety - now restricted to test environments only
- Added memory leak prevention in rate limiting middleware

### Added
- New `request_size_limit_middleware` for DoS protection
- Security documentation in `SECURITY.md`
- Admin panel with web interface for key management
- Redis support for persistent key state storage
- Comprehensive error handling and logging improvements

### Changed
- Updated Cargo.toml to use stable Rust 2021 edition
- Improved README documentation with current architecture
- Updated configuration examples with new options
- Enhanced logging with structured tracing and security-safe key previews

### Fixed
- Fixed integration tests by adding background worker for config updates
- Corrected file references in documentation
- Updated configuration examples to match current implementation

### Technical Improvements
- Added comprehensive test coverage (64+ tests)
- Improved error handling with structured error types
- Enhanced middleware stack with security layers
- Better separation of concerns in codebase architecture

## Previous Versions

This changelog was started with the security improvements update. For earlier changes, see the git commit history.