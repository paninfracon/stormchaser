use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    metrics::{PeriodicReader, SdkMeterProvider},
    runtime,
    trace::TracerProvider,
    Resource,
};
use std::env::var;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Initializes OpenTelemetry tracing, logging, and metrics based on environment variables.
pub fn init_telemetry(rust_log: &str) -> anyhow::Result<()> {
    let service_name =
        var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "stormchaser-engine".to_string());
    let otlp_endpoint =
        var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_else(|_| "http://localhost:4317".to_string());

    let resource = Resource::new(vec![KeyValue::new(
        opentelemetry_semantic_conventions::resource::SERVICE_NAME,
        service_name,
    )]);

    // Configure Tracing (Optional)
    let tracer = if var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok() {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&otlp_endpoint)
            .build()?;

        let tracer_provider = TracerProvider::builder()
            .with_batch_exporter(exporter, runtime::Tokio)
            .with_resource(resource.clone())
            .build();

        global::set_tracer_provider(tracer_provider.clone());

        use opentelemetry::trace::TracerProvider as _;
        Some(tracer_provider.tracer("stormchaser-engine"))
    } else {
        None
    };

    // Configure Metrics (Optional)
    if var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok() {
        let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(&otlp_endpoint)
            .build()?;

        let reader = PeriodicReader::builder(metric_exporter, runtime::Tokio).build();

        let meter_provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(resource)
            .build();

        global::set_meter_provider(meter_provider);
    }

    // Initialize Subscriber
    let env_filter = tracing_subscriber::EnvFilter::new(rust_log);

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer());

    if let Some(tracer) = tracer {
        registry
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .init();
    } else {
        registry.init();
    }

    Ok(())
}

/// Shutdown telemetry.
pub fn shutdown_telemetry() {
    global::shutdown_tracer_provider();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static INIT: Once = Once::new();

    #[test]
    fn test_init_telemetry_no_otel_endpoint() {
        // Ensure OTEL endpoint is NOT set for this test
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");

        // We can only init once per process safely
        INIT.call_once(|| {
            init_telemetry("debug").unwrap();
        });
    }
}
