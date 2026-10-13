use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    metrics::{PeriodicReader, SdkMeterProvider},
    trace::SdkTracerProvider,
    Resource,
};
use std::env::var;
use std::sync::OnceLock;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

static TRACER_PROVIDER: OnceLock<SdkTracerProvider> = OnceLock::new();
static METER_PROVIDER: OnceLock<SdkMeterProvider> = OnceLock::new();

/// Initializes OpenTelemetry tracing and metrics.
pub fn init_telemetry() -> anyhow::Result<()> {
    let service_name = var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "stormchaser-api".to_string());
    let otlp_endpoint =
        var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_else(|_| "http://localhost:4317".to_string());

    let resource = Resource::builder()
        .with_attribute(KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            service_name,
        ))
        .build();

    // Configure Tracing (Optional)
    let tracer = if var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok() {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&otlp_endpoint)
            .build()?;

        let tracer_provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(resource.clone())
            .build();

        global::set_tracer_provider(tracer_provider.clone());
        let _ = TRACER_PROVIDER.set(tracer_provider.clone());

        use opentelemetry::trace::TracerProvider as _;
        Some(tracer_provider.tracer("stormchaser-api"))
    } else {
        None
    };

    // Configure Metrics (Optional)
    if var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok() {
        let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(&otlp_endpoint)
            .build()?;

        let reader = PeriodicReader::builder(metric_exporter).build();

        let meter_provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(resource)
            .build();

        global::set_meter_provider(meter_provider.clone());
        let _ = METER_PROVIDER.set(meter_provider);
    }

    // Initialize Subscriber
    let env_filter =
        tracing_subscriber::EnvFilter::new(var("RUST_LOG").unwrap_or_else(|_| {
            "stormchaser_api=debug,tower_http=debug,axum::rejection=trace".into()
        }));

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

/// Shuts down the OpenTelemetry tracer provider, ensuring all telemetry is exported
pub fn shutdown_telemetry() {
    if let Some(provider) = TRACER_PROVIDER.get() {
        if let Err(e) = provider.shutdown() {
            tracing::warn!("Failed to shut down tracer provider: {}", e);
        }
    }
    if let Some(provider) = METER_PROVIDER.get() {
        if let Err(e) = provider.shutdown() {
            tracing::warn!("Failed to shut down meter provider: {}", e);
        }
    }
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
            init_telemetry().unwrap();
        });
    }
}
