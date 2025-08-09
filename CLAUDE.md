# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

JakeSky-rs is a Rust weather service that provides voice-friendly weather forecasts for Amazon Alexa. It supports multiple weather
providers (AccuWeather and OpenWeather) and can run both as a CLI application and AWS Lambda function.

## Common Commands

### Build and Test

- `cargo build` - Build the project
- `cargo fmt` - Format the source code
- `cargo test` - Run all tests
- `cargo check` - Check for compilation errors without building
- `cargo clippy -- -D warnings` - Run Rust linter for code quality checks

### Running the Application
- `cargo run --bin main -- --help` - Show CLI help
- `cargo run --bin main -- --latitude <lat> --longitude <lon> --api-key <key> --provider <provider>` - Run CLI
- `cargo run --bin lambda` - Run Lambda locally (requires environment variables)

### Target-Specific Commands (for AWS Lambda)
- `cargo build --target aarch64-unknown-linux-musl` - Build for Lambda deployment
- `cargo test --target aarch64-unknown-linux-musl` - Test with Lambda target
- `cargo clippy --target aarch64-unknown-linux-musl -- -D warnings` - Lint for Lambda target

## Architecture

### Binary Targets
- `main` (`src/main.rs`) - CLI application for local weather queries
- `lambda` (`src/lambda.rs`) - AWS Lambda function handler

### Core Modules
- `weather/mod.rs` - Weather provider abstraction and filtering logic
- `weather/accu_weather.rs` - AccuWeather API implementation
- `weather/open_weather.rs` - OpenWeather API implementation
- `alexa.rs` - Alexa response formatting and voice-friendly output generation

### Key Architecture Patterns
- Weather providers implement a common interface via the `WeatherProvider` enum
- The system filters hourly forecasts to specific times of interest (8am, 12pm, 6pm, optionally 10pm on weekends)
- Caching is implemented at the provider level using temporary files
- Lambda function handles AWS EventBridge warmup events
- All weather data is normalized to a common `Weather` struct regardless of provider

### Environment Variables
- `JAKESKY_API_KEY` - Weather provider API key
- `JAKESKY_LATITUDE` - Location latitude
- `JAKESKY_LONGITUDE` - Location longitude
