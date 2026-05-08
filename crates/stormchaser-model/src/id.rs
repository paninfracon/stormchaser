use nutype::nutype;
use schemars::{gen::SchemaGenerator, schema::Schema, JsonSchema};
use sqlx::{
    decode::Decode,
    encode::{Encode, IsNull},
    postgres::{PgArgumentBuffer, PgTypeInfo, PgValueRef},
    Postgres, Type,
};
use uuid::Uuid;

macro_rules! impl_id_traits {
    ($t:ident) => {
        impl Type<Postgres> for $t {
            fn type_info() -> PgTypeInfo {
                <Uuid as Type<Postgres>>::type_info()
            }
        }

        impl<'r> Decode<'r, Postgres> for $t {
            fn decode(value: PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
                let inner = <Uuid as Decode<'r, Postgres>>::decode(value)?;
                Ok($t::new(inner))
            }
        }

        impl<'q> Encode<'q, Postgres> for $t {
            fn encode_by_ref(
                &self,
                buf: &mut PgArgumentBuffer,
            ) -> Result<IsNull, sqlx::error::BoxDynError> {
                <Uuid as Encode<'q, Postgres>>::encode_by_ref(&self.into_inner(), buf)
            }
        }

        impl JsonSchema for $t {
            fn schema_name() -> String {
                stringify!($t).to_string()
            }
            fn json_schema(gen: &mut SchemaGenerator) -> Schema {
                <Uuid as JsonSchema>::json_schema(gen)
            }
        }

        impl utoipa::PartialSchema for $t {
            fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                utoipa::openapi::ObjectBuilder::new()
                    .schema_type(utoipa::openapi::schema::Type::String)
                    .format(Some(utoipa::openapi::schema::SchemaFormat::KnownFormat(
                        utoipa::openapi::schema::KnownFormat::Uuid,
                    )))
                    .build()
                    .into()
            }
        }

        impl utoipa::ToSchema for $t {
            fn name() -> std::borrow::Cow<'static, str> {
                std::borrow::Cow::Borrowed(stringify!($t))
            }
        }

        impl Default for $t {
            fn default() -> Self {
                Self::new(Uuid::nil())
            }
        }

        impl $t {
            pub fn new_v4() -> Self {
                Self::new(Uuid::new_v4())
            }
        }
    };
}

#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    FromStr
))]
pub struct RunId(Uuid);
impl_id_traits!(RunId);

#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    FromStr
))]
pub struct StepId(Uuid);
impl_id_traits!(StepId);

#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    FromStr
))]
pub struct StepInstanceId(Uuid);
impl_id_traits!(StepInstanceId);

#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    FromStr
))]
pub struct EventId(Uuid);
impl_id_traits!(EventId);

#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    FromStr
))]
pub struct RuleId(Uuid);
impl_id_traits!(RuleId);

#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    FromStr
))]
pub struct WebhookId(Uuid);
impl_id_traits!(WebhookId);

#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    FromStr
))]
pub struct BackendId(Uuid);
impl_id_traits!(BackendId);

#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    FromStr
))]
pub struct ArtifactId(Uuid);
impl_id_traits!(ArtifactId);

#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    FromStr
))]
pub struct TestReportId(Uuid);
impl_id_traits!(TestReportId);

#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    FromStr
))]
pub struct CronWorkflowId(Uuid);
impl_id_traits!(CronWorkflowId);

#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    FromStr
))]
pub struct StepDefinitionId(Uuid);
impl_id_traits!(StepDefinitionId);
