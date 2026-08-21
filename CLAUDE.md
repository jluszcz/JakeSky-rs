# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

JakeSky-rs is a Rust weather service that provides voice-friendly weather forecasts for Amazon Alexa. It supports multiple weather
providers (AccuWeather and OpenWeather) and can run both as a CLI application and AWS Lambda function.

## Common Commands

### Build and Test

- `cargo build` - Build the project
- `cargo fmt` - Format the source code
- `cargo fmt --check` - Check formatting
- `cargo test` - Run all tests
- `cargo check` - Check for compilation errors without building
- `cargo clippy --all-targets -- -D warnings` - Run Rust linter for code quality checks
- `pre-commit run --all-files` - Run the configured hooks (includes `cargo fmt --check`)

CI does not run these steps locally-style: `.github/workflows/ci.yml` is a thin caller of
`jluszcz/github-utils/.github/workflows/rust-ci.yml`, which runs build, test, `cargo fmt --check`, and
`cargo clippy --all-targets -- -D warnings` on `ubuntu-24.04-arm` against the
`aarch64-unknown-linux-musl` target. To reproduce a CI failure exactly, use the target-specific commands below.
On a push to `main`, CI additionally packages and deploys the Lambda via the shared `lambda-package` and
`deploy-lambda` workflows.

`.github/workflows/ci.yml` also calls the shared `terraform-ci.yml`, which runs `terraform fmt -check -recursive`, then `terraform init -backend=false` and `terraform validate` — the same checks the `terraform_fmt`/`terraform_validate` pre-commit hooks run locally. Terraform is never *applied* by CI.

The Terraform check is deliberately absent from `on.push.paths`. That filter gates the *whole* workflow, and `package`/`deploy` are gated only by `if: github.event_name == 'push'` — listing `.tf` there would make a Terraform-only push to `main` deploy the Lambda. The `pull_request` trigger has no path filter, so the check still runs on every PR, which is where it gates.

### Running the Application
- `cargo run --bin main -- --help` - Show CLI help
- `cargo run --bin main -- --latitude <lat> --longitude <lon> --api-key <key> --provider <provider>` - Run CLI
- `cargo run --bin lambda` - Run Lambda locally (requires environment variables)

### Target-Specific Commands (for AWS Lambda)
- `cargo build --target aarch64-unknown-linux-musl` - Build for Lambda deployment
- `cargo test --target aarch64-unknown-linux-musl` - Test with Lambda target
- `cargo clippy --target aarch64-unknown-linux-musl --all-targets -- -D warnings` - Lint for Lambda target

## Architecture

### Binary Targets
- `main` (`src/main.rs`) - CLI application for local weather queries
- `lambda` (`src/lambda.rs`) - AWS Lambda function handler

### Core Modules
- `weather/mod.rs` - Weather provider abstraction and filtering logic
- `weather/accu_weather.rs` - AccuWeather API implementation
- `weather/open_weather.rs` - OpenWeather API implementation
- `alexa.rs` - Alexa response formatting and voice-friendly output generation
- `alert_summary.rs` - Rule-based summarization of vague weather alerts
- `ai.rs` - Bedrock-backed fallback for the alerts the rules can't resolve

### Alert Summarization

NWS event names like "Special Weather Statement" say nothing useful out loud, so alerts go through two stages before
they are read:

1. **Rules first** (`alert_summary.rs`). `is_vague_event` identifies the useless event names; `extract_phenomenon`
   mines the alert *description* for what is actually being warned about.
2. **Bedrock only if the rules come up empty** (`ai.rs`). `summarizer_for` calls `needs_llm_fallback` and returns
   `None` when every alert already resolved — deliberately, so a run with no vague alerts never loads AWS config or
   credentials. `alexa::forecast` takes `Option<&S>` and falls back to the raw event name when it is `None` or the
   call fails.

Three constraints in `ai.rs` are load-bearing and easy to undo by accident:

- **`BEDROCK_TIMEOUT` is 2 seconds** against a 10-second Lambda timeout, because the response still has to be
  rendered afterwards. Failing fast into the event-name fallback beats a voice request that times out.
- **`MAX_SUMMARY_WORDS` caps what gets read aloud**, so a misbehaving model cannot dump a sentence into TTS.
- **The alert description is untrusted input.** It is NWS-provided but arrives over the network, and the prompt
  delimits it and tells the model not to follow instructions inside it. Keep that framing if you touch the prompt.

`BEDROCK_MODEL_ID` overrides the default model (`us.amazon.nova-2-lite-v1:0`), which is defined in
`jluszcz_rust_utils::bedrock::DEFAULT_MODEL_ID` rather than here — bumping it there re-points this Lambda too.

`BEDROCK_TIMEOUT` is applied by `BedrockClient::generate_with_timeout`; the timeout covers the Bedrock call, and
`clean_phrase` runs after it returns.

### Key Architecture Patterns
- Weather providers implement a common interface via the `WeatherProvider` enum
- The system filters hourly forecasts to specific times of interest: 8am, 12pm, and 6pm. `hours_of_interest` also
  supports adding 10pm on weekends, but **every production caller passes `add_weekend_hour: false`**
  (`weather/mod.rs`), so that behavior is currently reachable only from tests
- Caching is implemented at the provider level using temporary files
- Lambda function handles AWS EventBridge warmup events
- All weather data is normalized to a common `Weather` struct regardless of provider

### Environment Variables
- `JAKESKY_API_KEY` - Weather provider API key
- `JAKESKY_LATITUDE` - Location latitude
- `JAKESKY_LONGITUDE` - Location longitude
- `BEDROCK_MODEL_ID` - Overrides the alert-summarization model; default (`us.amazon.nova-2-lite-v1:0`) comes from rust-utils
