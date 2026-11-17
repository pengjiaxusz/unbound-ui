# Custom environment checks

`scripts/doctor.sh` (Unix) and `scripts/doctor.ps1` (Windows) first check the
tools the selected template layers use, then run every script in this
directory in sorted order.

## Contract

- Unix: `*.sh`, executed with `sh`.
- Windows: `*.ps1`, executed with `pwsh -NoProfile -File`.
- Working directory: the project root.
- `DOCTOR_ROOT`: the project root.
- `DOCTOR_FIX=1`: set when the doctor runs with `--fix` / `-Fix`. A custom
  check may repair its own tool in that case.
- Exit `0` means pass; any non-zero exit is reported as a failure and makes
  the doctor exit non-zero.

Keep each script self-contained. Do not install anything unless
`DOCTOR_FIX=1`, and print one short line describing what was found.

## Example (`scripts/doctor.d/10-cmake.sh`)

```sh
#!/bin/sh
if ! command -v cmake >/dev/null 2>&1; then
    echo "cmake not found; install it from https://cmake.org"
    exit 1
fi
echo "cmake $(cmake --version | head -n 1)"
```