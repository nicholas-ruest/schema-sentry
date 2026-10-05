# schema-sentry

`schema-sentry` is a small Rust CLI for catching backward-incompatible JSON
Schema changes before they break API clients, agent tools, or structured-output
consumers.

It answers one deliberately narrow question: **will the proposed new input
schema continue to accept everything the old schema accepted?** Findings carry
JSON paths and stable rule names, and the process exits with code `2` when it
finds a breaking change, making it useful as a CI gate.

## Install

```console
cargo install --git https://github.com/nicholas-ruest/schema-sentry
```

Or build locally with stable Rust:

```console
git clone https://github.com/nicholas-ruest/schema-sentry.git
cd schema-sentry
cargo build --release
```

## Use

```console
$ schema-sentry examples/old.json examples/new-breaking.json
incompatible (5 breaking change(s))
BREAKING $.limit [lower_bound_raised] numeric lower bound tightened from 1 to 5
BREAKING $.limit [property_became_required] property 'limit' is newly required
BREAKING $.limit [upper_bound_lowered] numeric upper bound tightened from 100 to 50
BREAKING $.mode [allowed_values_narrowed] new enum/const rejects: "fast"
BREAKING $.query [upper_bound_lowered] 'maxLength' was lowered from Some(500.0) to 200
$ echo $?
2
```

Machine-readable output is available for automation:

```console
schema-sentry old.json new.json --format json --allow-breaking
```

Exit codes are stable:

- `0`: compatible, or `--allow-breaking` was supplied
- `1`: input or execution error
- `2`: breaking changes found

## Checks

The current vertical slice understands the JSON Schema keywords that most often
shape tool and API inputs:

- `type`, `enum`, and `const`
- object `required`, `properties`, and `additionalProperties`
- array `items`, `minItems`, `maxItems`, and `uniqueItems`
- numeric bounds and `multipleOf`
- string `minLength`, `maxLength`, and `pattern`
- object `minProperties` and `maxProperties`
- boolean schemas

Composition, reference, and advanced applicator changes (`allOf`, `anyOf`,
`oneOf`, `$ref`, `prefixItems`, `patternProperties`, and conditionals) are
treated conservatively as breaking when they are added or changed, because this
tool would rather stop a release than silently claim compatibility it cannot
prove. `format` changes are warnings because validator enforcement varies.

Schema references are not resolved in v0.1.0; changing or adding one therefore
fails closed. Bundle or dereference schemas before comparing them. The
comparison is intentionally about accepted input sets; output-schema
compatibility has the opposite variance and is not yet modeled.

## Develop

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## License

Licensed under the MIT license.
