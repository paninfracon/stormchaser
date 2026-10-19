//! Per-object authorization: may this user act on this run, connection, webhook, event
//! rule or cron schedule?
//!
//! Request-level policy only sees a method and a path. Here the object is looked up
//! first, and policy is asked again with its owner and the caller's verified identity.
//! As with every policy check, nothing is enforced when no policy engine is configured.

use crate::auth::AuthClaims;
use crate::service::ServiceState;
use axum::{
    extract::{FromRequestParts, Request, State},
    http::{request::Parts, Method, StatusCode},
    middleware::Next,
    response::Response,
};
use sqlx::PgPool;
use std::collections::HashMap;
use stormchaser_model::auth::{Claims, OpaResource, ResourceOpaContext};
use uuid::Uuid;

/// The kinds of object that have an owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A workflow run, owned by whoever started it.
    Run,
    /// A connection.
    Connection,
    /// A webhook.
    Webhook,
    /// An event rule.
    EventRule,
    /// A cron schedule.
    CronWorkflow,
}

impl Kind {
    /// The name policy knows this kind by.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Run => "run",
            Kind::Connection => "connection",
            Kind::Webhook => "webhook",
            Kind::EventRule => "event_rule",
            Kind::CronWorkflow => "cron_workflow",
        }
    }

    /// The kind whose objects live under `/<segment>/{id}` in the API.
    fn from_path_segment(segment: &str) -> Option<Self> {
        match segment {
            "runs" => Some(Kind::Run),
            "connections" => Some(Kind::Connection),
            "webhooks" => Some(Kind::Webhook),
            "rules" => Some(Kind::EventRule),
            "cron-workflows" => Some(Kind::CronWorkflow),
            _ => None,
        }
    }

    /// The query that returns `(id, owner)` for the ids bound to `$1`.
    fn owners_query(self) -> &'static str {
        match self {
            Kind::Run => {
                r#"
                SELECT id, initiating_user FROM workflow_runs WHERE id = ANY($1)
                UNION ALL
                SELECT id, initiating_user FROM archived_workflow_runs WHERE id = ANY($1)
                "#
            }
            Kind::Connection => "SELECT id, created_by FROM connections WHERE id = ANY($1)",
            Kind::Webhook => "SELECT id, created_by FROM webhooks WHERE id = ANY($1)",
            Kind::EventRule => "SELECT id, created_by FROM event_rules WHERE id = ANY($1)",
            Kind::CronWorkflow => "SELECT id, created_by FROM cron_workflows WHERE id = ANY($1)",
        }
    }

    /// The statement that sets the owner (`$1`) of the object with id `$2`. A run
    /// records its initiating user when it is created.
    fn record_owner_statement(self) -> Option<&'static str> {
        match self {
            Kind::Run => None,
            Kind::Connection => Some("UPDATE connections SET created_by = $1 WHERE id = $2"),
            Kind::Webhook => Some("UPDATE webhooks SET created_by = $1 WHERE id = $2"),
            Kind::EventRule => Some("UPDATE event_rules SET created_by = $1 WHERE id = $2"),
            Kind::CronWorkflow => Some("UPDATE cron_workflows SET created_by = $1 WHERE id = $2"),
        }
    }
}

/// The authenticated caller, by the claims the API verified.
#[derive(Debug, Clone)]
pub struct Caller {
    /// Claims the API verified.
    pub claims: Claims,
}

impl Caller {
    /// The value recorded as the owner of what this caller creates: their email, or
    /// their subject id when the token carries no email.
    pub fn owner_key(&self) -> String {
        owner_key(&self.claims)
    }
}

/// The value recorded as the owner of what a user creates.
pub fn owner_key(claims: &Claims) -> String {
    claims.email.clone().unwrap_or_else(|| claims.sub.clone())
}

impl<S: ServiceState> FromRequestParts<S> for Caller {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let AuthClaims(claims) = AuthClaims::from_request_parts(parts, state).await?;
        Ok(Caller { claims })
    }
}

/// Asks policy whether `caller` may do `action` to an object.
pub async fn allowed<S: ServiceState>(
    state: &S,
    caller: &Caller,
    action: &str,
    kind: Kind,
    id: &str,
    owner: Option<&str>,
) -> Result<bool, StatusCode> {
    if !state.opa().is_configured() {
        return Ok(true);
    }
    let context = ResourceOpaContext {
        action,
        resource: OpaResource {
            kind: kind.as_str(),
            id,
            owner,
        },
        subject: caller.claims.opa_subject(),
    };
    state.opa().check_resource(context).await.map_err(|e| {
        tracing::error!(
            "Policy check failed for {} on {} {}: {:?}",
            action,
            kind.as_str(),
            id,
            e
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

/// Looks up the owners of some objects of one kind. Ids that do not exist are absent
/// from the result; objects with no owner map to `None`.
pub async fn owners(
    pool: &PgPool,
    kind: Kind,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Option<String>>, sqlx::Error> {
    let rows: Vec<(Uuid, Option<String>)> = sqlx::query_as(kind.owners_query())
        .bind(ids)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

/// The owner of one object, if it exists and has one.
pub async fn owner_of(pool: &PgPool, kind: Kind, id: Uuid) -> Option<String> {
    owners(pool, kind, &[id])
        .await
        .inspect_err(|e| {
            tracing::error!(
                "Failed to load the owner of {} {}: {:?}",
                kind.as_str(),
                id,
                e
            )
        })
        .ok()?
        .remove(&id)
        .flatten()
}

/// Records `caller` as the owner of an object they have just created. Run it in the
/// transaction that creates the object: committed without an owner, the object is
/// unowned, which policy lets everyone read and, until admins are configured, change.
pub async fn record_owner(
    executor: impl sqlx::PgExecutor<'_>,
    kind: Kind,
    id: Uuid,
    caller: &Caller,
) -> Result<(), StatusCode> {
    let Some(statement) = kind.record_owner_statement() else {
        return Ok(());
    };
    sqlx::query(statement)
        .bind(caller.owner_key())
        .bind(id)
        .execute(executor)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to record the owner of {} {}: {:?}",
                kind.as_str(),
                id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(())
}

/// Refuses with 403 unless `caller` may use the connection `id` (for example to point a
/// schedule or an event rule at its repository).
pub async fn authorize_connection_use<S: ServiceState>(
    state: &S,
    caller: &Caller,
    id: Uuid,
) -> Result<(), StatusCode> {
    authorize_action(state, caller, "use", Kind::Connection, id).await
}

/// The action of making a connection the default storage backend. Every run that names
/// no storage backend of its own keeps its files, artifacts and test reports there, so
/// whoever chooses it can read and replace other users' data.
pub const MAKE_DEFAULT: &str = "make_default";

/// Refuses unless `caller` may make the connection they are creating, `id`, the default
/// storage backend. An existing connection is checked with [`authorize_action`].
pub async fn authorize_make_default<S: ServiceState>(
    state: &S,
    caller: &Caller,
    id: Uuid,
) -> Result<(), StatusCode> {
    let owner = caller.owner_key();
    if allowed(
        state,
        caller,
        MAKE_DEFAULT,
        Kind::Connection,
        &id.to_string(),
        Some(&owner),
    )
    .await?
    {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

/// Refuses unless `caller` may do `action` to the object `id` of `kind`: 404 if it does
/// not exist, 403 if policy says no. For objects named in a request's body, which the
/// path-based check in [`authorize_object`] does not see.
pub async fn authorize_action<S: ServiceState>(
    state: &S,
    caller: &Caller,
    action: &str,
    kind: Kind,
    id: Uuid,
) -> Result<(), StatusCode> {
    if !state.opa().is_configured() {
        return Ok(());
    }
    let owner = owners(state.pool(), kind, &[id])
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to load the owner of {} {}: {:?}",
                kind.as_str(),
                id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .remove(&id)
        .ok_or(StatusCode::NOT_FOUND)?;
    if allowed(
        state,
        caller,
        action,
        kind,
        &id.to_string(),
        owner.as_deref(),
    )
    .await?
    {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

/// Decides which objects of one kind a caller may read, asking policy once per distinct
/// owner instead of once per object.
pub struct ReadFilter<'a, S: ServiceState> {
    state: &'a S,
    caller: &'a Caller,
    kind: Kind,
    decided: HashMap<Option<String>, bool>,
}

impl<'a, S: ServiceState> ReadFilter<'a, S> {
    /// Starts a filter for `caller` over objects of `kind`.
    pub fn new(state: &'a S, caller: &'a Caller, kind: Kind) -> Self {
        Self {
            state,
            caller,
            kind,
            decided: HashMap::new(),
        }
    }

    /// True when the caller may read an object owned by `owner`.
    pub async fn can_read(&mut self, owner: Option<&str>) -> Result<bool, StatusCode> {
        if !self.state.opa().is_configured() {
            return Ok(true);
        }
        let key = owner.map(str::to_owned);
        if let Some(decision) = self.decided.get(&key) {
            return Ok(*decision);
        }
        let decision = allowed(self.state, self.caller, "read", self.kind, "", owner).await?;
        self.decided.insert(key, decision);
        Ok(decision)
    }
}

/// Drops from a listing the objects the caller may not read, so that a list never shows
/// what fetching the object would refuse.
pub async fn filter_readable<S: ServiceState, T>(
    state: &S,
    caller: &Caller,
    kind: Kind,
    items: Vec<T>,
    id_of: impl Fn(&T) -> Uuid,
) -> Result<Vec<T>, StatusCode> {
    if !state.opa().is_configured() || items.is_empty() {
        return Ok(items);
    }
    let ids: Vec<Uuid> = items.iter().map(&id_of).collect();
    let owners = owners(state.pool(), kind, &ids).await.map_err(|e| {
        tracing::error!("Failed to load owners of {}s: {:?}", kind.as_str(), e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut filter = ReadFilter::new(state, caller, kind);
    let mut readable = Vec::with_capacity(items.len());
    for item in items {
        let owner = owners.get(&id_of(&item)).cloned().flatten();
        if filter.can_read(owner.as_deref()).await? {
            readable.push(item);
        }
    }
    Ok(readable)
}

/// What a request does to the object its path names, if it names one.
///
/// `path` is relative to `/api/v1` and still percent-encoded, as it is in the request URI.
/// Each segment is decoded before it is read, the same way the `Path` extractor decodes it
/// for the handler: otherwise `/runs/<id with %2D>` would fail to parse here and skip the
/// check, while the handler acted on the decoded id. A segment that is not valid UTF-8
/// decodes lossily, so it cannot parse here and the extractor rejects it too.
///
/// Approving or rejecting a step is left to the approval policy, which exists precisely so
/// that someone other than the owner can do it.
fn object_action(method: &Method, path: &str) -> Option<(Kind, Uuid, &'static str)> {
    let segments: Vec<String> = path
        .trim_matches('/')
        .split('/')
        .map(|s| String::from_utf8_lossy(&urlencoding::decode_binary(s.as_bytes())).into_owned())
        .collect();
    let mut segments = segments.iter().map(String::as_str);
    let kind = Kind::from_path_segment(segments.next()?)?;
    let id = Uuid::parse_str(segments.next()?).ok()?;
    let rest: Vec<&str> = segments.collect();

    if kind == Kind::Run && matches!(rest.as_slice(), ["steps", _, "approve" | "reject"]) {
        return None;
    }
    let action = match *method {
        Method::GET | Method::HEAD | Method::OPTIONS => "read",
        Method::DELETE if rest.is_empty() => "delete",
        _ => "update",
    };
    Some((kind, id, action))
}

/// Middleware that authorizes every request whose path names an object by id.
///
/// It sits in front of the whole protected route group, so a new route under, say,
/// `/runs/{id}/` is covered without its handler doing anything. An object that does not
/// exist is left to the handler, which answers 404.
pub async fn authorize_object<S: ServiceState>(
    State(state): State<S>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if !state.opa().is_configured() {
        return Ok(next.run(request).await);
    }
    let path = request.uri().path();
    let path = path.strip_prefix("/api/v1").unwrap_or(path);
    let Some((kind, id, action)) = object_action(request.method(), path) else {
        return Ok(next.run(request).await);
    };

    let owner = owners(state.pool(), kind, &[id]).await.map_err(|e| {
        tracing::error!(
            "Failed to load the owner of {} {}: {:?}",
            kind.as_str(),
            id,
            e
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let Some(owner) = owner.get(&id) else {
        return Ok(next.run(request).await);
    };

    let AuthClaims(claims) = request
        .extensions()
        .get::<AuthClaims>()
        .cloned()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let caller = Caller { claims };

    if allowed(
        &state,
        &caller,
        action,
        kind,
        &id.to_string(),
        owner.as_deref(),
    )
    .await?
    {
        Ok(next.run(request).await)
    } else {
        tracing::info!(
            "Policy denied {} on {} {} to {}",
            action,
            kind.as_str(),
            id,
            caller.owner_key()
        );
        Err(StatusCode::FORBIDDEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_that_name_an_object_map_to_an_action() {
        let id = Uuid::new_v4();
        let get = |p: String| object_action(&Method::GET, &p);

        assert_eq!(get(format!("/runs/{id}")), Some((Kind::Run, id, "read")));
        assert_eq!(
            get(format!("/runs/{id}/steps/{}/logs", Uuid::new_v4())),
            Some((Kind::Run, id, "read"))
        );
        assert_eq!(
            object_action(&Method::DELETE, &format!("/runs/{id}")),
            Some((Kind::Run, id, "delete"))
        );
        assert_eq!(
            object_action(&Method::PATCH, &format!("/connections/{id}")),
            Some((Kind::Connection, id, "update"))
        );
        assert_eq!(
            object_action(&Method::DELETE, &format!("/cron-workflows/{id}/")),
            Some((Kind::CronWorkflow, id, "delete"))
        );
        assert_eq!(
            object_action(&Method::DELETE, &format!("/rules/{id}")),
            Some((Kind::EventRule, id, "delete"))
        );
        // Anything else done to an object counts as changing it
        assert_eq!(
            object_action(&Method::POST, &format!("/webhooks/{id}/rotate")),
            Some((Kind::Webhook, id, "update"))
        );
    }

    #[test]
    fn percent_encoded_paths_are_checked_as_the_handler_sees_them() {
        let id = Uuid::new_v4();
        let encoded = id.to_string().replacen('-', "%2D", 1);
        assert_eq!(
            object_action(&Method::PATCH, &format!("/connections/{encoded}")),
            Some((Kind::Connection, id, "update"))
        );
        assert_eq!(
            object_action(&Method::GET, &format!("/%72uns/{id}")),
            Some((Kind::Run, id, "read"))
        );
        // An encoded "approve" is still an approval, and an encoded "steps" is not
        let step = Uuid::new_v4();
        assert_eq!(
            object_action(&Method::POST, &format!("/runs/{id}/steps/{step}/%61pprove")),
            None
        );
        assert_eq!(
            object_action(&Method::POST, &format!("/runs/{id}/st%65ps/{step}/x")),
            Some((Kind::Run, id, "update"))
        );
    }

    #[test]
    fn collection_and_approval_paths_are_not_object_checks() {
        let id = Uuid::new_v4();
        let step = Uuid::new_v4();
        assert_eq!(object_action(&Method::GET, "/runs"), None);
        assert_eq!(object_action(&Method::GET, "/runs/stream"), None);
        assert_eq!(object_action(&Method::POST, "/runs/direct"), None);
        assert_eq!(object_action(&Method::POST, "/connections/test"), None);
        assert_eq!(object_action(&Method::POST, "/schema/parse-git"), None);
        assert_eq!(
            object_action(&Method::POST, &format!("/runs/{id}/steps/{step}/approve")),
            None
        );
        assert_eq!(
            object_action(&Method::POST, &format!("/runs/{id}/steps/{step}/reject")),
            None
        );
    }
}
