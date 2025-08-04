# Gemini Proxy Refactoring Plan

## Current Issues Identified

1. **Large monolithic modules**: `key_manager.rs` is 650+ lines with multiple responsibilities
2. **Mixed concerns**: State management, Redis operations, and business logic are intertwined
3. **Duplicate code**: Similar patterns in Redis and in-memory stores
4. **Complex error handling**: Error types could be more specific
5. **Configuration complexity**: Large config struct with optional fields
6. **Testing challenges**: Tightly coupled components make unit testing difficult

## Proposed Structure

```
src/
├── core/                    # Core business logic
│   ├── mod.rs
│   ├── key_rotation.rs      # Key rotation algorithms
│   ├── circuit_breaker.rs   # Circuit breaker logic (existing)
│   └── health_check.rs      # Health checking logic
├── storage/                 # Data persistence layer
│   ├── mod.rs
│   ├── traits.rs           # Storage traits
│   ├── redis.rs            # Redis implementation
│   ├── memory.rs           # In-memory implementation
│   └── key_state.rs        # Key state management
├── handlers/               # HTTP handlers (existing structure is good)
├── middleware/             # Middleware (existing structure is good)
├── config/                 # Configuration management
│   ├── mod.rs
│   ├── app.rs              # Main app config
│   ├── validation.rs       # Config validation
│   └── loader.rs           # Config loading logic
├── metrics/                # Metrics and observability
│   ├── mod.rs
│   ├── collector.rs
│   └── prometheus.rs
└── utils/                  # Utility functions
    ├── mod.rs
    ├── crypto.rs           # Cryptographic utilities
    └── http.rs             # HTTP utilities
```

## Refactoring Steps

### Phase 1: Extract Storage Layer
- Create storage traits for key management
- Separate Redis and in-memory implementations
- Add proper error handling for storage operations

### Phase 2: Simplify Key Manager
- Extract key rotation logic
- Reduce responsibilities to coordination only
- Improve testability

### Phase 3: Configuration Refactoring
- Split large config struct into smaller, focused structs
- Add builder pattern for complex configurations
- Improve validation logic

### Phase 4: Error Handling Improvements
- Create domain-specific error types
- Implement proper error context
- Add error recovery strategies

### Phase 5: Performance Optimizations
- Reduce Arc/RwLock usage where possible
- Implement connection pooling improvements
- Add caching layer for frequently accessed data