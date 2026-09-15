# Changelog

## 0.2.0

This release adds configuration composition, source provenance, selected environment
overlays, and redacted parser diagnostics. It is a breaking release relative to 0.1.6.

### Migration from 0.1.x

- Update `einstellung` to `"0.2.0"`. If you depend directly on
  `einstellung_derive`, update it to `"0.2.0"` too. Normally the `derive` feature
  supplies the macro without a separate dependency.
- `ConfigError::Json`, `Toml`, and `Yaml` now contain the library's `JsonError`,
  `TomlError`, and `YamlError` wrappers. These suppress potentially sensitive parser
  output in `Display` and `Debug`. Use `error.source_location()` for line/column
  information, or the wrapper's `parser_error()` for deliberate access to backend
  diagnostics. Replace direct JSON/TOML variant construction with `error.into()`;
  the backend error conversions remain available.
- YAML now uses `serde-saphyr` instead of `serde_yaml`. Code that names
  `serde_yaml::Error` must adapt to the new backend or use `ConfigError` instead.
  Test application YAML fixtures when upgrading, especially enum tags and malformed
  input; duplicate mapping keys and multiple documents are rejected.
- `ConfigError` can now cross thread boundaries. Validation and custom-merge
  reasons use `BoxError` (`Box<dyn Error + Send + Sync + 'static>`). Update helpers
  returning `Box<dyn Error>` to return `einstellung::BoxError`, and make custom
  error types thread-safe. Validator and merge errors must convert into `BoxError`.
- Loads through `Config` and builders attach source/path/provenance context.
  Match on `error.root_cause()` when inspecting variants such as `MissingField`,
  rather than matching only the outer error. `logical_path()`, `config_source()`,
  and `field_sources()` expose the contextual information.
- Logical paths use Serde's deserialization names, including `rename` and
  `rename_all`, rather than always using Rust field names. Flattened subconfig
  paths omit the containing Rust field. Update path-based diagnostic consumers.
- Invalid or ambiguous derive combinations now fail explicitly: generic structs,
  duplicate deserialized field names, defaults on optional fields, flatten without
  subconfig, defaults on flattened subconfigs, and strict unknown-field checking
  combined with flatten. Put flattened defaults on nested fields instead.
- Invalid flattened subconfig values now propagate deserialization errors instead
  of being treated as absent. For typed values behind flattening or untagged enums
  in string providers, use `KeyValueProvider::with_json` or the environment/dotenv
  `with_json_var` mapping. Ordinary mappings retain literal string semantics when
  Serde buffers them before learning the destination type.
- The core supports Rust 1.85, `derive` requires 1.88, and `yaml` requires 1.89.
  Default features therefore require Rust 1.89. Use `default-features = false`
  when selecting a smaller feature set for an older compiler.
- Default features remain JSON, TOML, YAML, and derive. `full` additionally enables
  the new environment and dotenv providers; these are opt-in otherwise.

### Added

- `ConfigBuilder` composes providers in precedence order, supports named and
  preloaded layers, stops loading after failures, and applies field defaults at
  final construction.
- Tracked complete and partial configurations preserve field supply histories and
  successfully merged layers, including across staged composition. Provenance
  stores paths and source labels, not configuration values.
- `KeyValueProvider`, allowlist/prefix-based `EnvProvider`, and `DotenvProvider`.
  Dotenv loads do not mutate process environment; `without_substitution()` rejects
  variable expansion for isolated file loading.
- `Secret<T>` redacts display/debug output and requires `expose_secret()` for
  explicit access. It deliberately does not implement `Serialize`.
- Runtime format selection with `FormatProvider`, owned provider constructors,
  and heterogeneous providers through the object-safe `ConfigProviderFor<C>`.
- Mode-specific `ConfigView` conversion, `require_for_view`, reusable validators,
  strict unknown-field checking, and optional/flattened subconfigs.
- Structured parser errors retain nested field, map, and collection-index paths.
- Explicit JSON-valued mappings support typed flattened fields, untagged enums,
  and non-unit enum representations without guessing the type of ordinary strings.

### Release validation

- Restored the advertised Rust 1.85 core build by avoiding let-chain syntax.
- Expression defaults are evaluated only when needed. Required flattened defaults
  work without a provider; absent optional flattened objects add no phantom default
  provenance. Raw struct identifiers and empty freezable structs derive correctly.
- README examples compile as all-feature doctests; the layered example imports
  `ConfigProvider` explicitly.
- Both crate archives include MIT and Apache license texts. Package verification
  checks their contents against the workspace originals and verifies the main
  crate against the exact packaged derive artifact.
- The package preflight resolves dependencies with `--locked` to reject stale
  lockfiles before applying the temporary derive-artifact patch.
- Packaged README doctests and both runnable examples are verified from the
  extracted archive. Example fixture paths work in the published crate layout,
  and loading failures exit unsuccessfully rather than only printing an error.
- Compiler diagnostic snapshots are refreshed for Rust 1.98. Runtime CI also runs
  on Windows and macOS, alongside the Linux feature and MSRV checks.
