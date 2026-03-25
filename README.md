**scratchgen — Image Grid Generator**

- Binary CLI (Rust) that composes a square N×N image grid from a pool of smaller images. Build with `cargo build --release` (binary name follows the crate name in `Cargo.toml`).

Usage
- Build: `cargo build --release`
- Generate example placeholder images: `./target/release/scratchgen --generate-examples --output out/placeholder.png` (writes `images/flowers/*` and `images/animals/*`).
- Run with a config: `./target/release/scratchgen --config configs/example_constrained.json --output out/grid_constrained.png --seed 42`.

Requirements
- Rust toolchain to build (or use a prebuilt binary).
- The program reads common image formats (png, jpg, webp, tiff).

CLI options
- `--config <FILE>`: path to JSON or YAML config.
- `--pool <PATTERN>...`: override config `pool` (one or more glob patterns).
- `--mode <MODE>`: `constrained` (default), `independent`, or `without_replacement`.
- `--tile-size <N>`: pixel size of each tile (default `256`). Final image size is `tile_size * grid_size` square.
- `--grid-size <N>`: dimension `N` for an N x N grid (minimum `2`, default `3`).
- `--fit <FIT>`: `cover` (default), `contain`, or `stretch` (how to resize input images).
- `--seed <N>`: deterministic RNG seed (0 = random/entropy).
- `--output <FILE>`: required output path (PNG/JPEG inferred by extension).
- `--background <COLOR>`: hex `#rrggbb` background (default `#000000`).
- `--allow-repeat-when-pool-small`: when sampling without replacement, allow repeats after exhausting pool.
- `--generate-examples`: create sample images under `images/flowers` and `images/animals` and exit.

Config file fields (JSON/YAML)
- `pool` (required unless `--pool` used): array of glob patterns or directories, e.g. `["images/flowers/*.jpg", "images/animals/*.png"]`.
- `weights` (optional): map of `path_or_basename -> number` controlling sampling weight.
- `min_count` / `max_count` (optional): maps to enforce minimum/maximum occurrences per-image in `constrained` mode.
- `mode`, `tile_size`, `fit`, `seed`, `output`, `background`, `grid_size`, `allow_repeat_when_pool_small` are optional config fields and are overridden by corresponding CLI flags when provided.

Selection modes explained
- `constrained`: compute expected counts per image = `weight / total_weight * (N*N)`, take `floor(expected)` as base and distribute remaining slots by largest remainders; enforce `min_count`/`max_count` by clamping and redistributing. Guarantees counts sum to `N*N` and respect min/max. Final multiset is shuffled deterministically by `seed`.
- `independent`: sample each of the `N*N` tiles independently with replacement using weights. No per-image guarantees (only statistical expectation).
- `without_replacement`: weighted sampling without replacement. If `pool` size < `N*N` and `allow_repeat_when_pool_small` is false the CLI returns an error; if true, after exhausting the pool sampling may restart.

Image sizing / fit
- `cover`: center-crop to preserve aspect ratio and fill the tile (recommended).
- `contain`: scale to fit inside tile and pad with `background` color.
- `stretch`: ignore aspect ratio and resize exactly to tile size.

Determinism
- Use `--seed` to produce reproducible outputs. The program uses a ChaCha RNG seeded from the provided value.

Return values and errors
- On success the program exits `0` and prints a JSON metadata object to stdout containing `seed`, `mode`, `tiles`, and `output`.
- On error the program exits non-zero and prints an error message.

Examples
- Generate example images and produce a constrained grid:

  ```sh
  ./target/release/scratchgen --generate-examples --output out/placeholder.png
  ./target/release/scratchgen --config configs/example_constrained.json --output out/grid_constrained.png --seed 42
  ```

- Independent sampling with explicit pool override:

  ```sh
  ./target/release/scratchgen --pool "images/*" --mode independent --tile-size 256 --output out/grid_independent.png --seed 123
  ```

PHP integration (calling from PHP via exec)

Use `escapeshellarg()` for arguments that may contain spaces. The CLI prints a JSON metadata object to stdout which can be parsed by the caller.

```php
$bin = '/path/to/scratchgen';
$cmd = $bin . ' --config ' . escapeshellarg('/app/imgen.json') . ' --output ' . escapeshellarg('/tmp/grid.png') . ' --seed 42';
exec($cmd . ' 2>&1', $out, $rc);
if ($rc !== 0) {
    // handle error: $out contains stderr output
} else {
    // /tmp/grid.png created; $out contains any printed metadata
}
```

Files and examples in this repo
- `configs/example_constrained.json`, `configs/example_independent.json`, `configs/example_without_replacement.json` — example configs.
- `images/flowers/*`, `images/animals/*` — example images created by `--generate-examples`.

Next actions you can request
1) Add YAML equivalents for the example configs.
2) Add unit tests for allocation and deterministic sampling.
3) Produce a statically-linked musl release binary for easy deployment.

License: MIT
