# JakeSky-rs

Returns the current weather, as well as a forecast for 8am, 12pm, and 6pm for the current date. Only upcoming forecast times are included (e.g. if it's already past noon, the 8am and 12pm entries are omitted).

## Status

[![Status Badge](https://github.com/jluszcz/JakeSky-rs/actions/workflows/build-and-deploy.yml/badge.svg)](https://github.com/jluszcz/JakeSky-rs/actions/workflows/build-and-deploy.yml)

## Supported Weather Providers

- [AccuWeather](https://www.accuweather.com)
- [OpenWeather](https://openweathermap.org)

## Usage

### CLI

```sh
cargo run --bin main -- \
  --latitude <lat> \
  --longitude <lon> \
  --api-key <key> \
  --provider <accuweather|openweather>
```

Options can also be provided via environment variables:

| Flag | Environment Variable | Default |
|---|---|---|
| `--api-key` | `JAKESKY_API_KEY` | *(required)* |
| `--latitude` | `JAKESKY_LATITUDE` | *(required)* |
| `--longitude` | `JAKESKY_LONGITUDE` | *(required)* |
| `--provider` | — | `openweather` |

### AWS Lambda

The Lambda function uses OpenWeather and reads configuration from the following environment variables:

- `JAKESKY_API_KEY`
- `JAKESKY_LATITUDE`
- `JAKESKY_LONGITUDE`

It handles AWS EventBridge warmup events automatically.

#### Building for Lambda

```sh
cargo build --target aarch64-unknown-linux-musl
```
