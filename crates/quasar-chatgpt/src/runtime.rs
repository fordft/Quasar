//! One native session owner. Login attempts and active request epochs are separate.
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use tokio::sync::watch;

use crate::oauth::{Authorization, Client};
use crate::store::Lease;
use crate::{now, random, Error, Model, Profile, Secret, SessionView, State, Store};

/// Reuse an account registration, or explicitly create a separate one.
#[derive(Clone, Debug, Default)]
pub struct LoginOptions {
    pub profile_id: Option<String>,
    pub new_profile: bool,
    pub consent: bool,
}

struct Pending {
    sequence: u64,
    cancel: watch::Sender<bool>,
}

/// Long-lived native owner, shared by account UI and model credential suppliers.
pub struct Runtime {
    store: Store,
    client: Client,
    mutations: tokio::sync::Mutex<()>,
    pending: Mutex<Option<Pending>>,
    sequence: std::sync::atomic::AtomicU64,
    epoch: watch::Sender<u64>,
    fault: Mutex<Option<Error>>,
    rejected: std::sync::atomic::AtomicBool,
    active: Mutex<Option<String>>,
    signed_out: std::sync::atomic::AtomicBool,
}

impl Runtime {
    pub fn new(store: Store) -> Result<Arc<Self>, Error> {
        let active = {
            let lease = store.lease()?;
            store.read(&lease)?.active
        };
        let (epoch, _) = watch::channel(0);
        Ok(Arc::new(Self {
            store,
            active: Mutex::new(active),
            signed_out: std::sync::atomic::AtomicBool::new(false),
            client: Client::new()?,
            mutations: tokio::sync::Mutex::new(()),
            pending: Mutex::new(None),
            sequence: std::sync::atomic::AtomicU64::new(0),
            epoch,
            fault: Mutex::new(None),
            rejected: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    /// A request's account generation. It changes on login, switch and logout.
    pub fn epoch(&self) -> u64 {
        *self.epoch.borrow()
    }

    /// Model streams use this to stop when their original account is revoked.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.epoch.subscribe()
    }

    fn advance(&self) {
        self.epoch
            .send_modify(|epoch| *epoch = epoch.saturating_add(1));
    }

    // A cooperating process may switch the durable active registration. Never
    // let an existing client silently authenticate as that different account.
    fn observe_active(&self, state: &State) -> bool {
        let mut active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        if *active == state.active {
            return false;
        }
        *active = state.active.clone();
        self.advance();
        true
    }

    fn set_fault(&self, fault: Option<Error>) {
        *self.fault.lock().unwrap_or_else(|p| p.into_inner()) = fault;
    }

    async fn load(&self) -> Result<(Lease, State), Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let lease = store.lease()?;
            let state = store.read(&lease)?;
            Ok((lease, state))
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    async fn save(&self, lease: Lease, state: State) -> Result<State, Error> {
        self.checkpoint(lease, state).await.map(|(_, state)| state)
    }

    // Keep the cross-process lease through checkpoint verification and activation.
    async fn checkpoint(&self, lease: Lease, state: State) -> Result<(Lease, State), Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            store.write(&lease, &state)?;
            Ok((lease, state))
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    pub async fn session(&self) -> Result<SessionView, Error> {
        let _mutation = self.mutations.lock().await;
        let (_lease, state) = self.load().await?;
        self.observe_active(&state);
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some();
        let fault = *self.fault.lock().unwrap_or_else(|p| p.into_inner());
        let mut view = state.view(pending, fault);
        if self.signed_out.load(std::sync::atomic::Ordering::SeqCst) {
            let active = view.active.clone();
            if let Some(profile) = view
                .profiles
                .iter_mut()
                .find(|p| Some(&p.id) == active.as_ref())
            {
                profile.sharing = false;
            }
        }
        Ok(view)
    }

    pub async fn begin(&self, options: LoginOptions) -> Result<Authorization, Error> {
        let _mutation = self.mutations.lock().await;
        if self
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
        {
            return Err(Error::Busy);
        }
        let (lease, state) = self.load().await?;
        let selected = if options.new_profile {
            if options.profile_id.is_some() || state.profiles.len() >= crate::MAX_PROFILES {
                return Err(Error::Protocol);
            }
            None
        } else {
            let wanted = options
                .profile_id
                .as_ref()
                .or(state.active.as_ref())
                .or_else(|| {
                    state
                        .profiles
                        .iter()
                        .rev()
                        .find(|p| p.identity.is_none())
                        .map(|p| &p.id)
                });
            match wanted {
                Some(wanted) => Some(
                    state
                        .profiles
                        .iter()
                        .find(|p| &p.id == wanted)
                        .cloned()
                        .ok_or(Error::Protocol)?,
                ),
                None => None,
            }
        };
        let profile = selected
            .as_ref()
            .map(|p| p.id.clone())
            .unwrap_or(URL_SAFE_NO_PAD.encode(random::<16>()?));
        // Persist the installation host before the browser can authorize it.
        let state = self.save(lease, state).await?;
        let sequence = self
            .sequence
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        let (cancel, receiver) = watch::channel(false);
        let authorization = Authorization::start(
            &state.host_id,
            profile,
            selected,
            options.consent,
            sequence,
            receiver,
        )
        .await?;
        *self.pending.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(Pending { sequence, cancel });
        let blocked = *self.fault.lock().unwrap_or_else(|p| p.into_inner())
            == Some(Error::PlanPermissionRequired);
        if !blocked {
            self.set_fault(None);
        }
        Ok(authorization)
    }

    fn current(&self, sequence: u64) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|p| p.sequence == sequence)
    }

    /// Completes the loopback flow. Registration is saved before code exchange.
    pub async fn finish(&self, mut authorization: Authorization) -> Result<(), Error> {
        let sequence = authorization.epoch;
        let mut cancelled = authorization.cancellation();
        let already_cancelled = *cancelled.borrow();
        let result = if already_cancelled {
            Err(Error::Cancelled)
        } else {
            tokio::select! {
                biased;
                _ = cancelled.changed() => Err(Error::Cancelled),
                result = self.complete(&mut authorization) => result,
            }
        };
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        if pending.as_ref().is_some_and(|p| p.sequence == sequence) {
            *pending = None;
            self.set_fault(
                result
                    .as_ref()
                    .err()
                    .copied()
                    .filter(|e| *e != Error::Cancelled),
            );
        }
        result
    }

    async fn complete(&self, authorization: &mut Authorization) -> Result<(), Error> {
        let callback = authorization.callback().await?;
        {
            let _mutation = self.mutations.lock().await;
            if !self.current(authorization.epoch) {
                return Err(Error::Cancelled);
            }
            let (lease, mut state) = self.load().await?;
            if let Some(profile) = state
                .profiles
                .iter()
                .find(|p| p.id == authorization.profile)
            {
                if profile.client_id != callback.client_id {
                    return Err(Error::InvalidIdentity);
                }
            } else {
                if state.profiles.len() >= crate::MAX_PROFILES {
                    return Err(Error::Protocol);
                }
                if state
                    .profiles
                    .iter()
                    .any(|p| p.client_id == callback.client_id)
                {
                    return Err(Error::InvalidIdentity);
                }
                state.profiles.push(Profile {
                    id: authorization.profile.clone(),
                    client_id: callback.client_id.clone(),
                    identity: None,
                    tokens: None,
                    pending_refresh: None,
                });
            }
            self.save(lease, state).await?;
        }
        let (identity, tokens) = self.client.exchange(authorization, &callback).await?;
        let _mutation = self.mutations.lock().await;
        if !self.current(authorization.epoch) {
            return Err(Error::Cancelled);
        }
        let (lease, mut state) = self.load().await?;
        let profile = state
            .profiles
            .iter_mut()
            .find(|p| p.id == authorization.profile)
            .ok_or(Error::InvalidIdentity)?;
        if profile.client_id != callback.client_id
            || profile
                .identity
                .as_ref()
                .is_some_and(|old| old.issuer != identity.issuer || old.subject != identity.subject)
        {
            return Err(Error::InvalidIdentity);
        }
        profile.identity = Some(identity);
        profile.tokens = Some(tokens);
        profile.pending_refresh = None;
        state.active = Some(profile.id.clone());
        let state = self.save(lease, state).await?;
        *self.active.lock().unwrap_or_else(|p| p.into_inner()) = state.active;
        self.signed_out
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.rejected
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.advance();
        Ok(())
    }

    pub fn cancel(&self) {
        if let Some(pending) = self
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            pending.cancel.send_replace(true);
        }
    }

    /// Makes a verified saved connection active, without mixing registrations.
    pub async fn select(&self, id: &str) -> Result<(), Error> {
        self.cancel();
        let _mutation = self.mutations.lock().await;
        let (lease, mut state) = self.load().await?;
        if !state
            .profiles
            .iter()
            .any(|p| p.id == id && p.identity.is_some() && p.tokens.is_some())
        {
            return Err(Error::SignedOut);
        }
        state.active = Some(id.to_owned());
        let state = self.save(lease, state).await?;
        *self.active.lock().unwrap_or_else(|p| p.into_inner()) = state.active;
        self.signed_out
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.rejected
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.set_fault(None);
        self.advance();
        Ok(())
    }

    /// Stop requests and clear local tokens before bounded remote revocation.
    pub async fn sign_out(&self) -> Result<(), Error> {
        self.signed_out
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.cancel();
        self.advance();
        let _mutation = self.mutations.lock().await;
        let (lease, mut state) = self.load().await?;
        if self.observe_active(&state) {
            return Err(Error::SignedOut);
        }
        let Some(profile) = state
            .active
            .as_ref()
            .and_then(|id| state.profiles.iter_mut().find(|p| &p.id == id))
        else {
            return Ok(());
        };
        let revocation = profile.clone();
        profile.tokens = None;
        profile.pending_refresh = None;
        // Local erasure survives cancellation or shutdown during the network call.
        self.save(lease, state).await?;
        let confirmed = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.client.revoke(&revocation),
        )
        .await
        .unwrap_or(false);
        let fault = (!confirmed).then_some(Error::RemoteRevocationUnconfirmed);
        self.set_fault(fault);
        fault.map_or(Ok(()), Err)
    }

    /// A provider rejected the current access token. The next request refreshes.
    pub fn rejected(&self) {
        self.rejected
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Rotation is owned by the session, even when its caller cancels a stream.
    pub async fn access_token(self: &Arc<Self>, expected_epoch: u64) -> Result<Secret, Error> {
        let owner = self.clone();
        tokio::spawn(async move { owner.access_token_inner(expected_epoch).await })
            .await
            .map_err(|_| Error::Storage)?
    }

    async fn access_token_inner(&self, expected_epoch: u64) -> Result<Secret, Error> {
        if self.signed_out.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(Error::SignedOut);
        }
        if self.epoch() != expected_epoch {
            return Err(Error::SignedOut);
        }
        let _mutation = self.mutations.lock().await;
        if self.epoch() != expected_epoch {
            return Err(Error::SignedOut);
        }
        let blocked = *self.fault.lock().unwrap_or_else(|p| p.into_inner())
            == Some(Error::PlanPermissionRequired);
        if blocked {
            return Err(Error::PlanPermissionRequired);
        }
        let (lease, mut state) = self.load().await?;
        if self.observe_active(&state) || self.epoch() != expected_epoch {
            return Err(Error::SignedOut);
        }
        let index = state
            .active
            .as_ref()
            .and_then(|id| state.profiles.iter().position(|p| &p.id == id))
            .ok_or(Error::SignedOut)?;
        if state.profiles[index].pending_refresh.is_some() {
            return self
                .commit_rotation(lease, state, index, expected_epoch)
                .await;
        }
        let profile = &state.profiles[index];
        let tokens = profile.tokens.as_ref().ok_or(Error::SignedOut)?;
        if !tokens.sharing() {
            return Err(Error::PlanPermissionRequired);
        }
        let time = now()?;
        let rejected = self.rejected.load(std::sync::atomic::Ordering::SeqCst);
        let refresh_allowed = tokens.refresh_allowed(time);
        if !rejected
            && tokens.expires_at > time
            && (tokens.expires_at > time.saturating_add(120)
                || !refresh_allowed
                || tokens.refresh.is_none())
        {
            return tokens.bearer();
        }
        if !refresh_allowed {
            return Err(Error::RetryLater);
        }
        match self.client.rotate(profile).await {
            Ok((tokens, verify)) => {
                state.profiles[index].pending_refresh = Some(crate::PendingRefresh {
                    tokens,
                    received_at: self.client.received_at()?,
                    verify_identity: verify,
                });
                // Persist the only valid successor before fetching rotating keys.
                // It remains quarantined until its new identity is verified.
                let (lease, state) = self.checkpoint(lease, state).await?;
                self.commit_rotation(lease, state, index, expected_epoch)
                    .await
            }
            Err(error @ (Error::SignedOut | Error::InvalidIdentity | Error::Protocol)) => {
                state.profiles[index].tokens = None;
                state.profiles[index].pending_refresh = None;
                self.save(lease, state).await?;
                self.set_fault(Some(error));
                self.advance();
                Err(error)
            }
            Err(error) => {
                if !rejected
                    && tokens.expires_at > time
                    && matches!(error, Error::Network | Error::RateLimited)
                {
                    tokens.bearer()
                } else {
                    Err(error)
                }
            }
        }
    }

    async fn commit_rotation(
        &self,
        lease: Lease,
        mut state: State,
        index: usize,
        epoch: u64,
    ) -> Result<Secret, Error> {
        let profile = &state.profiles[index];
        let pending = profile.pending_refresh.as_ref().ok_or(Error::Protocol)?;
        if pending.verify_identity {
            match self.client.verify_rotation(profile, pending).await {
                Ok(identity) => state.profiles[index].identity = Some(identity),
                Err(Error::InvalidIdentity) => {
                    state.profiles[index].tokens = None;
                    state.profiles[index].pending_refresh = None;
                    self.save(lease, state).await?;
                    self.set_fault(Some(Error::InvalidIdentity));
                    self.advance();
                    return Err(Error::InvalidIdentity);
                }
                Err(error) => {
                    self.set_fault(Some(error));
                    return Err(error);
                }
            }
        }
        let pending = state.profiles[index]
            .pending_refresh
            .take()
            .ok_or(Error::Protocol)?;
        let sharing = pending.tokens.sharing();
        let expired = pending.tokens.expires_at <= now()?;
        let bearer = if sharing && !expired {
            Some(pending.tokens.bearer()?)
        } else {
            None
        };
        state.profiles[index].tokens = Some(pending.tokens);
        self.save(lease, state).await?;
        self.rejected
            .store(false, std::sync::atomic::Ordering::SeqCst);
        if !sharing {
            self.set_fault(Some(Error::PlanPermissionRequired));
            self.advance();
            return Err(Error::PlanPermissionRequired);
        }
        if self.signed_out.load(std::sync::atomic::Ordering::SeqCst) || self.epoch() != epoch {
            return Err(Error::SignedOut);
        }
        self.set_fault(None);
        bearer.ok_or(Error::RetryLater)
    }

    pub async fn models(self: &Arc<Self>) -> Result<Vec<Model>, Error> {
        let epoch = self.epoch();
        let result = async {
            let token = self.access_token(epoch).await?;
            match self.client.models(&token).await {
                Err(Error::SignedOut) => {
                    self.rejected();
                    let token = self.access_token(epoch).await?;
                    self.client.models(&token).await
                }
                result => result,
            }
        }
        .await;
        if self.epoch() != epoch {
            return Err(Error::SignedOut);
        }
        match &result {
            Err(Error::PlanPermissionRequired) => {
                self.set_fault(Some(Error::PlanPermissionRequired));
                self.advance();
            }
            Err(Error::SignedOut) => {
                let _mutation = self.mutations.lock().await;
                if self.epoch() != epoch {
                    return Err(Error::SignedOut);
                }
                let (lease, mut state) = self.load().await?;
                if self.observe_active(&state) {
                    return Err(Error::SignedOut);
                }
                if let Some(profile) = state
                    .active
                    .as_ref()
                    .and_then(|id| state.profiles.iter_mut().find(|p| &p.id == id))
                {
                    profile.tokens = None;
                    profile.pending_refresh = None;
                }
                self.save(lease, state).await?;
                self.set_fault(Some(Error::SignedOut));
                self.advance();
            }
            Err(error) => self.set_fault(Some(*error)),
            Ok(_) => self.set_fault(None),
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    struct Key;
    impl crate::EncryptionKey for Key {
        fn load(&self, _: &str, _: bool) -> Result<Zeroizing<Vec<u8>>, Error> {
            Ok(Zeroizing::new(vec![19; 32]))
        }
    }

    #[tokio::test]
    async fn host_persists_and_cancelled_login_does_not_become_an_account() {
        let directory = tempfile::tempdir().expect("temp");
        let store = Store::new(directory.path().join("auth"), Arc::new(Key)).expect("store");
        let runtime = Runtime::new(store.clone()).expect("runtime");
        let attempt = runtime.begin(LoginOptions::default()).await.expect("begin");
        let url = url::Url::parse(attempt.url()).expect("url");
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        let host = query.get("ext_agent_host_id").expect("host").clone();
        assert_eq!(
            query.get("client_id").map(String::as_str),
            Some("dynamic_agent_client")
        );
        assert_eq!(
            query.get("agent_name_hint").map(String::as_str),
            Some("Quasar")
        );
        assert!(!format!("{attempt:?}").contains("state="));
        runtime.cancel();
        assert_eq!(runtime.finish(attempt).await, Err(Error::Cancelled));
        assert!(runtime
            .session()
            .await
            .expect("session")
            .profiles
            .is_empty());
        let second = Runtime::new(store).expect("restart");
        let attempt = second.begin(LoginOptions::default()).await.expect("again");
        let url = url::Url::parse(attempt.url()).expect("url");
        assert!(url
            .query_pairs()
            .any(|(key, value)| key == "ext_agent_host_id" && value == host));
        second.cancel();
        assert_eq!(
            second.access_token(second.epoch()).await.err(),
            Some(Error::SignedOut)
        );
    }

    #[test]
    fn identity_sign_in_does_not_grant_plan_usage() {
        let tokens = crate::Tokens {
            access: Some(Zeroizing::new("access".into())),
            refresh: None,
            id: Zeroizing::new("id".into()),
            scopes: vec!["openid".into()],
            expires_at: 1000,
            earliest_refresh_at: None,
        };
        assert!(!tokens.sharing());
        assert!(!format!(
            "{:?}",
            Secret::new("private-test-bearer".into()).expect("secret")
        )
        .contains("private-test-bearer"));
    }
    fn seeded(expired: bool, refresh: bool) -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().expect("directory");
        let store = Store::new(directory.path().join("auth"), Arc::new(Key)).expect("store");
        let lease = store.lease().expect("lease");
        let mut state = State::fresh().expect("state");
        for id in ["one", "two"] {
            state.profiles.push(Profile {
                id: id.into(),
                client_id: format!("oaiapp_{id}"),
                identity: Some(crate::Identity {
                    issuer: crate::ISSUER.into(),
                    subject: id.into(),
                    email: Some("same@example.invalid".into()),
                    name: None,
                }),
                pending_refresh: None,
                tokens: Some(crate::Tokens {
                    access: Some(Zeroizing::new(format!("test-access-{id}"))),
                    refresh: refresh.then(|| Zeroizing::new(format!("test-refresh-{id}"))),
                    id: Zeroizing::new("test-id-hint".into()),
                    scopes: vec!["resource.invoke".into(), "chatgpt.tokens.use.direct".into()],
                    expires_at: if expired {
                        now().expect("time") - 1
                    } else {
                        now().expect("time") + 3600
                    },
                    earliest_refresh_at: None,
                }),
            });
        }
        state.active = Some("one".into());
        store.write(&lease, &state).expect("write");
        drop(lease);
        (directory, store)
    }
    fn fixture_runtime(store: Store, server: &crate::fixtures::Server) -> Arc<Runtime> {
        let mut runtime = Runtime::new(store).expect("runtime");
        Arc::get_mut(&mut runtime).expect("unshared runtime").client =
            Client::fixture(server.base.clone());
        runtime
    }
    fn rotation(scope: &str) -> serde_json::Value {
        serde_json::json!({ "access_token": "test-rotated-access", "refresh_token": "test-rotated-refresh",
            "token_type": "Bearer", "expires_in": 3600, "scope": scope })
    }
    const SHARING: &str = "openid resource.invoke chatgpt.tokens.use.direct";

    #[tokio::test]
    async fn concurrent_owners_refresh_once_and_commit_the_rotated_tuple() {
        let (_dir, store) = seeded(true, true);
        let server = crate::fixtures::Server::start(vec![(200, rotation(SHARING))]).await;
        let first = fixture_runtime(store.clone(), &server);
        let second = fixture_runtime(store.clone(), &server);
        let (a, b) = tokio::join!(first.access_token(0), second.access_token(0));
        assert_eq!(a.expect("first").expose(), "test-rotated-access");
        assert_eq!(b.expect("second").expose(), "test-rotated-access");
        let requests = server.requests.lock().expect("requests");
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("refresh_token=test-refresh-one"));
        assert!(requests[0].contains("client_id=oaiapp_one"));
        let lease = store.lease().expect("lease");
        let state = store.read(&lease).expect("durable");
        let tokens = state.profiles[0].tokens.as_ref().expect("tokens");
        assert_eq!(
            tokens.refresh.as_deref().map(String::as_str),
            Some("test-rotated-refresh")
        );
        assert!(tokens.expires_at > now().expect("now"));
        assert!(tokens.sharing());
    }

    #[tokio::test]
    async fn invalid_grant_and_lost_plan_permission_close_ai_access() {
        for (status, response, expected, signed_in) in [
            (
                400,
                serde_json::json!({"error": "invalid_grant"}),
                Error::SignedOut,
                false,
            ),
            (
                200,
                rotation("openid email"),
                Error::PlanPermissionRequired,
                true,
            ),
        ] {
            let (_dir, store) = seeded(true, true);
            let server = crate::fixtures::Server::start(vec![(status, response)]).await;
            let runtime = fixture_runtime(store, &server);
            assert_eq!(runtime.access_token(0).await.err(), Some(expected));
            let view = runtime.session().await.expect("session");
            assert!(!view.sharing());
            assert_eq!(view.active().expect("profile").signed_in, signed_in);
            assert_ne!(runtime.epoch(), 0);
        }
    }

    #[tokio::test]
    async fn an_old_client_cannot_send_as_a_switched_account_even_across_owners() {
        let (_dir, store) = seeded(false, false);
        let first = Runtime::new(store.clone()).expect("first");
        let second = Runtime::new(store).expect("second");
        let mut changes = first.subscribe();
        assert_eq!(
            first.access_token(0).await.expect("first bearer").expose(),
            "test-access-one"
        );
        second.select("two").await.expect("switch");
        assert_eq!(first.access_token(0).await.err(), Some(Error::SignedOut));
        changes.changed().await.expect("account epoch");
        let view = first.session().await.expect("view");
        assert_eq!(view.active.as_deref(), Some("two"));
        assert_eq!(view.profiles[0].email, view.profiles[1].email);
    }

    #[tokio::test]
    async fn models_retry_one_unauthorized_get_and_filter_account_discovery() {
        let (_dir, store) = seeded(false, true);
        let server = crate::fixtures::Server::start(vec![
            (401, serde_json::json!({})),
            (200, rotation(SHARING)),
            (
                200,
                serde_json::json!({"models": [
                    {"slug": "listed", "display_name": "Listed", "visibility": "list"},
                    {"slug": "hidden", "display_name": "Hidden", "visibility": "hide"},
                    {"slug": "listed", "display_name": "Duplicate", "visibility": "list"}
                ]}),
            ),
        ])
        .await;
        let runtime = fixture_runtime(store, &server);
        let models = runtime.models().await.expect("models");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].slug, "listed");
        let requests = server.requests.lock().expect("requests");
        assert_eq!(requests.len(), 3);
        assert!(requests[0]
            .to_lowercase()
            .contains("authorization: bearer test-access-one"));
        assert!(requests[2]
            .to_lowercase()
            .contains("authorization: bearer test-rotated-access"));
    }

    #[tokio::test]
    async fn sign_out_stops_requests_first_and_clears_tokens_when_revocation_fails() {
        let (_dir, store) = seeded(false, true);
        let server = crate::fixtures::Server::start(vec![
            (200, serde_json::json!({"issuer": crate::ISSUER, "revocation_endpoint": "https://auth.openai.com/api/accounts/oauth/revoke"})),
            (500, serde_json::json!({})), (500, serde_json::json!({})), (500, serde_json::json!({})),
        ]).await;
        let runtime = fixture_runtime(store.clone(), &server);
        let mut changes = runtime.subscribe();
        let work = runtime.clone();
        let signing_out = tokio::spawn(async move { work.sign_out().await });
        tokio::time::timeout(std::time::Duration::from_secs(1), changes.changed())
            .await
            .expect("immediate cancellation")
            .expect("epoch");
        assert_eq!(runtime.access_token(0).await.err(), Some(Error::SignedOut));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while server.requests.lock().expect("requests").is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("revocation started");
        {
            let lease = store.lease().expect("lease during revocation");
            assert!(store.read(&lease).expect("local state").profiles[0]
                .tokens
                .is_none());
        }

        assert_eq!(
            signing_out.await.expect("task"),
            Err(Error::RemoteRevocationUnconfirmed)
        );
        let view = runtime.session().await.expect("view");
        assert!(!view.sharing());
        assert!(!view.active().expect("registration").signed_in);
        let lease = store.lease().expect("lease");
        let state = store.read(&lease).expect("state");
        assert!(state.profiles[0].tokens.is_none());
        assert_eq!(state.profiles[0].client_id, "oaiapp_one");
        let requests = server.requests.lock().expect("requests");
        assert_eq!(requests.len(), 4);
        assert!(requests[1].contains("token_type_hint=refresh_token"));
        assert!(requests[1].contains("token=test-refresh-one"));
    }
    #[tokio::test]
    async fn cancelling_during_exchange_preserves_only_the_issued_registration_for_retry() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let directory = tempfile::tempdir().expect("directory");
        let store = Store::new(directory.path().join("auth"), Arc::new(Key)).expect("store");
        let server = crate::fixtures::Server::delayed(
            vec![(400, serde_json::json!({"error": "cancelled_fixture"}))],
            std::time::Duration::from_secs(2),
        )
        .await;
        let runtime = fixture_runtime(store.clone(), &server);
        let authorization = runtime.begin(LoginOptions::default()).await.expect("begin");
        let query: std::collections::HashMap<_, _> = url::Url::parse(authorization.url())
            .expect("url")
            .query_pairs()
            .into_owned()
            .collect();
        let redirect = url::Url::parse(&query["redirect_uri"]).expect("redirect");
        let owner = runtime.clone();
        let finishing = tokio::spawn(async move { owner.finish(authorization).await });
        let host = format!("127.0.0.1:{}", redirect.port().expect("port"));
        let mut socket = tokio::net::TcpStream::connect(&host)
            .await
            .expect("callback connect");
        let callback = format!("GET /auth/callback?code=test-code&state={}&client_id=oaiapp_cancelled_fixture HTTP/1.1\r\nHost: {host}\r\n\r\n", query["state"]);
        socket
            .write_all(callback.as_bytes())
            .await
            .expect("callback write");
        let mut response = vec![0; 2048];
        let read = socket.read(&mut response).await.expect("callback response");
        assert!(std::str::from_utf8(&response[..read])
            .expect("response")
            .starts_with("HTTP/1.1 200"));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while server.requests.lock().expect("requests").is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("exchange started");
        runtime.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), finishing)
            .await
            .expect("cancellation does not await HTTP")
            .expect("task");
        assert_eq!(result, Err(Error::Cancelled));
        let view = runtime.session().await.expect("view");
        assert_eq!(view.profiles.len(), 1);
        assert!(!view.profiles[0].signed_in);
        let retry = runtime.begin(LoginOptions::default()).await.expect("retry");
        let retry = url::Url::parse(retry.url()).expect("retry URL");
        assert!(retry
            .query_pairs()
            .any(|(key, value)| key == "client_id" && value == "oaiapp_cancelled_fixture"));
        runtime.cancel();
    }

    #[tokio::test]
    async fn resource_permission_refusal_keeps_identity_but_stops_model_credentials() {
        let (_dir, store) = seeded(false, true);
        let server = crate::fixtures::Server::start(vec![(403, serde_json::json!({}))]).await;
        let runtime = fixture_runtime(store, &server);
        assert_eq!(
            runtime.models().await.err(),
            Some(Error::PlanPermissionRequired)
        );
        let view = runtime.session().await.expect("session");
        assert!(view.active().expect("identity").signed_in);
        assert!(!view.sharing());
        assert_eq!(
            runtime.access_token(runtime.epoch()).await.err(),
            Some(Error::PlanPermissionRequired)
        );
        assert_eq!(server.requests.lock().expect("requests").len(), 1);
    }
    #[tokio::test]
    async fn a_failed_local_erasure_cannot_reenable_ai_with_the_old_bearer() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct FailsOnWrite(AtomicUsize);
        impl crate::EncryptionKey for FailsOnWrite {
            fn load(&self, _: &str, _: bool) -> Result<Zeroizing<Vec<u8>>, Error> {
                if self.0.fetch_add(1, Ordering::SeqCst) == 1 {
                    return Err(Error::Storage);
                }
                Ok(Zeroizing::new(vec![19; 32]))
            }
        }
        let (directory, original) = seeded(false, false);
        let key = Arc::new(FailsOnWrite(AtomicUsize::new(0)));
        let store = Store::new(directory.path().join("auth"), key.clone()).expect("faulting store");
        let runtime = Runtime::new(store).expect("runtime");
        key.0.store(0, Ordering::SeqCst);
        assert_eq!(runtime.sign_out().await, Err(Error::Storage));
        assert_eq!(
            runtime.access_token(runtime.epoch()).await.err(),
            Some(Error::SignedOut)
        );
        let lease = original.lease().expect("lease");
        assert!(
            original.read(&lease).expect("durable old record").profiles[0]
                .tokens
                .is_some()
        );
        drop(lease);
        assert!(!runtime.session().await.expect("view").sharing());
    }

    #[tokio::test]
    async fn cancelling_a_caller_does_not_discard_a_rotated_refresh_token() {
        let (_dir, store) = seeded(true, true);
        let server = crate::fixtures::Server::delayed(
            vec![(200, rotation(SHARING))],
            std::time::Duration::from_millis(150),
        )
        .await;
        let runtime = fixture_runtime(store.clone(), &server);
        let owner = runtime.clone();
        let caller = tokio::spawn(async move { owner.access_token(0).await });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while server.requests.lock().expect("requests").is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("refresh started");
        caller.abort();
        assert_eq!(
            runtime.access_token(0).await.expect("successor").expose(),
            "test-rotated-access"
        );
        assert_eq!(server.requests.lock().expect("requests").len(), 1);
        let lease = store.lease().expect("lease");
        let state = store.read(&lease).expect("saved successor");
        assert!(state.profiles[0].pending_refresh.is_none());
        assert_eq!(
            state.profiles[0]
                .tokens
                .as_ref()
                .expect("tokens")
                .refresh
                .as_deref()
                .map(String::as_str),
            Some("test-rotated-refresh")
        );
    }

    #[tokio::test]
    async fn key_service_outages_resume_a_quarantined_rotation_after_restart() {
        let vector: serde_json::Value =
            serde_json::from_str(include_str!("oidc-test-vector.json")).expect("vector");
        let mut malformed = vector["jwks"].clone();
        malformed["keys"][0]["n"] = serde_json::json!("!");
        for unavailable in [
            (503, serde_json::json!({})),
            (200, serde_json::json!({"keys": []})),
            (200, malformed),
        ] {
            let (_dir, store) = seeded(true, true);
            {
                let lease = store.lease().expect("lease");
                let mut state = store.read(&lease).expect("state");
                state.profiles[0].client_id = "oaiapp_quasar_unit_test".into();
                state.profiles[0]
                    .identity
                    .as_mut()
                    .expect("identity")
                    .subject = "test-subject".into();
                store.write(&lease, &state).expect("fixture identity");
            }
            let mut response = rotation(SHARING);
            response["id_token"] = vector["token"].clone();
            let server = crate::fixtures::Server::start(vec![
                (200, response),
                unavailable,
                (200, vector["jwks"].clone()),
            ])
            .await;
            let runtime = fixture_runtime(store.clone(), &server);
            assert_eq!(runtime.access_token(0).await.err(), Some(Error::Network));
            assert!(!runtime.session().await.expect("quarantined").sharing());
            {
                let lease = store.lease().expect("lease");
                let state = store.read(&lease).expect("checkpoint");
                let pending = state.profiles[0]
                    .pending_refresh
                    .as_ref()
                    .expect("durable rotation");
                assert_eq!(
                    pending.tokens.refresh.as_deref().map(String::as_str),
                    Some("test-rotated-refresh")
                );
                assert_eq!(pending.received_at, 1800000030);
            }
            drop(runtime);
            let restarted = fixture_runtime(store.clone(), &server);
            assert_eq!(
                restarted.access_token(0).await.expect("recovered").expose(),
                "test-rotated-access"
            );
            let requests = server.requests.lock().expect("requests");
            assert_eq!(requests.len(), 3);
            assert!(requests[0].starts_with("POST "));
            assert!(requests[1].starts_with("GET "));
            assert!(requests[2].starts_with("GET "));
        }
    }

    #[tokio::test]
    async fn a_refreshed_identity_must_keep_its_subject_and_valid_signature() {
        let vector: serde_json::Value =
            serde_json::from_str(include_str!("oidc-test-vector.json")).expect("vector");
        for bad_signature in [false, true] {
            let (_dir, store) = seeded(true, true);
            {
                let lease = store.lease().expect("lease");
                let mut state = store.read(&lease).expect("state");
                state.profiles[0].client_id = "oaiapp_quasar_unit_test".into();
                if bad_signature {
                    state.profiles[0]
                        .identity
                        .as_mut()
                        .expect("identity")
                        .subject = "test-subject".into();
                }
                store.write(&lease, &state).expect("fixture");
            }
            let mut response = rotation(SHARING);
            let mut token = vector["token"].as_str().expect("token").as_bytes().to_vec();
            if bad_signature {
                let offset = token.iter().rposition(|b| *b == b'.').expect("signature") + 1;
                token[offset] = if token[offset] == b'A' { b'B' } else { b'A' };
            }
            response["id_token"] = serde_json::json!(String::from_utf8(token).expect("token"));
            let server = crate::fixtures::Server::start(vec![
                (200, response),
                (200, vector["jwks"].clone()),
            ])
            .await;
            let runtime = fixture_runtime(store.clone(), &server);
            assert_eq!(
                runtime.access_token(0).await.err(),
                Some(Error::InvalidIdentity)
            );
            assert_ne!(runtime.epoch(), 0);
            let lease = store.lease().expect("lease");
            let state = store.read(&lease).expect("cleared");
            assert!(state.profiles[0].tokens.is_none());
            assert!(state.profiles[0].pending_refresh.is_none());
        }
    }

    #[tokio::test]
    async fn an_expired_token_waits_until_the_provider_allows_refresh() {
        let (_dir, store) = seeded(true, true);
        {
            let lease = store.lease().expect("lease");
            let mut state = store.read(&lease).expect("state");
            state.profiles[0]
                .tokens
                .as_mut()
                .expect("tokens")
                .earliest_refresh_at = Some(serde_json::json!(now().expect("now") + 60));
            store.write(&lease, &state).expect("future refresh");
        }
        let server = crate::fixtures::Server::start(vec![]).await;
        let runtime = fixture_runtime(store, &server);
        assert_eq!(runtime.access_token(0).await.err(), Some(Error::RetryLater));
        assert!(server.requests.lock().expect("requests").is_empty());
    }

    #[tokio::test]
    async fn a_rejected_bearer_stays_blocked_while_refresh_is_not_ready() {
        let (_dir, store) = seeded(false, true);
        {
            let lease = store.lease().expect("lease");
            let mut state = store.read(&lease).expect("state");
            state.profiles[0]
                .tokens
                .as_mut()
                .expect("tokens")
                .earliest_refresh_at = Some(serde_json::json!(now().expect("now") + 60));
            store.write(&lease, &state).expect("future refresh");
        }
        let server = crate::fixtures::Server::start(vec![]).await;
        let runtime = fixture_runtime(store, &server);
        runtime.rejected();
        for _ in 0..2 {
            assert_eq!(runtime.access_token(0).await.err(), Some(Error::RetryLater));
        }
        assert!(server.requests.lock().expect("requests").is_empty());
    }

    #[tokio::test]
    async fn sign_out_revokes_the_pending_successor_and_erases_both_generations() {
        let (_dir, store) = seeded(true, true);
        {
            let lease = store.lease().expect("lease");
            let mut state = store.read(&lease).expect("state");
            let mut tokens = state.profiles[0].tokens.clone().expect("tokens");
            tokens.refresh = Some(Zeroizing::new("test-pending-successor".into()));
            state.profiles[0].pending_refresh = Some(crate::PendingRefresh {
                tokens,
                received_at: now().expect("now"),
                verify_identity: true,
            });
            store.write(&lease, &state).expect("checkpoint");
        }
        let server = crate::fixtures::Server::start(vec![
            (200, serde_json::json!({"issuer": crate::ISSUER, "revocation_endpoint": "https://auth.openai.com/api/accounts/oauth/revoke"})),
            (200, serde_json::json!({})),
        ]).await;
        let runtime = fixture_runtime(store.clone(), &server);
        runtime.sign_out().await.expect("sign out");
        let requests = server.requests.lock().expect("requests");
        assert!(requests[1].contains("token=test-pending-successor"));
        assert!(!requests[1].contains("test-refresh-one"));
        let lease = store.lease().expect("lease");
        let state = store.read(&lease).expect("local erasure");
        assert!(state.profiles[0].tokens.is_none());
        assert!(state.profiles[0].pending_refresh.is_none());
    }
}
