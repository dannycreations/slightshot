# Slightshot Development Guide

## Commands

| Command                         | What it does                              | Speed                     |
| ------------------------------- | ----------------------------------------- | ------------------------- |
| `make check`                    | Compiles and lints the entire project     | Slow                      |
| `make check -- -p package_name` | Compiles and lints one specific package   | Fast                      |
| `make test`                     | Runs all tests in the entire project      | Very slow                 |
| `make test -- -p package_name`  | Runs all tests in one specific package    | Slow                      |
| `make test my_test_case`        | Runs a single test, module, or test group | Fast                      |
| `make bench`                    | Runs all performance benchmarks           | Very slow — use sparingly |

---

## Guidelines

### 1. Never edit `[profile.*]` sections in `Cargo.toml`

Sections like `[profile.dev]` and `[profile.release]` control compiler optimization settings for the whole project. They are locked because changing them can silently break **reproducibility** — the guarantee that the same code always produces the same build, on any machine, at any time.

```toml
# ✅ Allowed — adding a new dependency
[dependencies]
serde = { version = "1", features = ["derive"] }

# ❌ Forbidden — modifying any [profile.*] block
[profile.release]
opt-level = 3
```

---

### 2. Use `make test`, not `cargo test`

`make test` includes safety protections that `cargo test` does not — for example, automatic timeouts that stop a test run if it hangs. Running `cargo test` directly skips these protections entirely.

```sh
# ✅ Correct
make test
make test -- -p package_name
make test my_test_case

# ❌ Forbidden — bypasses timeout and safety protections
cargo test
cargo test -p package_name
cargo test my_test_case
```

---

### 3. Avoid `unsafe` code

`unsafe` blocks disable Rust's normal compile-time safety checks. Only use `unsafe` in these two cases:

- **FFI (Foreign Function Interface):** interacting with non-Rust code, such as a C library.
- **Performance-critical code:** only after profiling has proven a measurable speed benefit.

Every `unsafe` block **must** be preceded by a `// SAFETY:` comment. This comment must explain why the code is safe (what conditions or guarantees make it so) and, where possible, link to supporting documentation.

```rust
// ✅ Permitted — FFI usage with a documented safety justification
// SAFETY: `ptr` is guaranteed non-null and valid for `len` bytes
//   by the C caller contract in ffi_contract.md §3.2.
unsafe {
  std::slice::from_raw_parts(ptr, len)
}

// ❌ Forbidden — no SAFETY comment, no explanation
unsafe {
  *raw_ptr = 42;
}
```

---

### 4. Keep all imports at the module level and use imported names throughout the file

All dependencies used by a Rust module must be imported with `use` declarations at the **top level of the file**. The import section should be the single, explicit place where the module's external and internal dependencies are declared.

**Do not:**

- Put `use` declarations inside functions, methods, closures, `impl` blocks, traits, or any other nested scope.
- Use inline qualified paths such as `std::collections::HashMap`, `crate::foo::Bar`, or `serde::Serialize` in implementation code when the referenced item can be imported at the top level.
- Repeat a dependency's module path throughout the implementation.
- Hide dependencies inside local scopes.

**Do:**

- Add every required type, trait, function, macro, enum variant, and module item to the file's top-level `use` section.
- Use the imported name directly throughout the rest of the file.
- Keep the complete dependency surface of the file visible from its top-level imports.

```rust
// CORRECT: all dependencies are declared at the top level.
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::foo::Bar;

pub fn build_index(items: &[&str], state: &Bar) -> HashMap<&str, usize> {
  items
    .iter()
    .enumerate()
    .map(|(i, k)| (*k, i))
    .collect()
}

// FORBIDDEN: dependency referenced through a qualified path inline.
pub fn build_index(
  items: &[&str],
  state: &crate::foo::Bar,
) -> std::collections::HashMap<&str, usize> {
  // FORBIDDEN: local import inside a function.
  use serde::{Deserialize, Serialize};

  items
    .iter()
    .enumerate()
    .map(|(i, k)| (*k, i))
    .collect()
}
```
