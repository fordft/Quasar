//! One explicit approval of the fixed disposable-account fixture. The offer is
//! process-local and never a model tool, durable grant, or run-wide permission.
use zephium_core::work::{runtime::*, WorkExecutionId, WorkStepId};

pub(super) const MESSAGE: &str = "Quasar Work qualification: approved send and extraction test.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Decision {
    pub offer: u32,
    pub approve: bool,
}

fn approved_output(bytes: &[u8]) -> bool {
    bytes == b"approved\n"
}

/// The native dialog receives the full confirmation over stdin, never argv or
/// logs. Its fixed script interprets JSON as data and returns one closed word.
const DIALOG: &str = r#"function run() {
  ObjC.import('Foundation');
  try {
    const raw = $.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;
    const body = JSON.parse(ObjC.unwrap($.NSString.alloc.initWithDataEncoding(raw, $.NSUTF8StringEncoding)));
    if (typeof body !== 'string') return 'declined';
    const app = Application.currentApplication(); app.includeStandardAdditions = true;
    const answer = app.displayDialog(body, {withTitle: 'Quasar Work test approval', buttons: ['Decline', 'Approve once'], defaultButton: 'Decline', cancelButton: 'Decline', givingUpAfter: 90});
    return !answer.gaveUp && answer.buttonReturned === 'Approve once' ? 'approved' : 'declined';
  } catch (_) { return 'declined'; }
}"#;

fn dialog_body(start: &str, confirm: &WorkConfirmV1) -> Option<Vec<u8>> {
    if !Gate::eligible(confirm)
        || start.len() > 1024
        || confirm.facts.len() > 12
        || confirm.headline.len() > 300
        || confirm.action.len() > 300
        || confirm
            .facts
            .iter()
            .any(|fact| fact.label.len() > 300 || fact.value.len() > 300)
    {
        return None;
    }
    let mut body = format!(
        "Disposable-account qualification only. Review the destination and exact message. Approve once only if they match your intended test channel.\n\nRequested channel: {start}\n\nPage-provided confirmation:\n{}\nAction: {}\n",
        confirm.headline, confirm.action
    );
    for fact in &confirm.facts {
        body.push_str(&format!("{}: {}\n", fact.label, fact.value));
    }
    body.push_str(&format!("\nExact message:\n{MESSAGE}\n\nThis grants only this one held action. Closing or timing out declines it."));
    let bytes = serde_json::to_vec(&body).ok()?;
    (bytes.len() <= 16 * 1024).then_some(bytes)
}

async fn native_review(body: Vec<u8>) -> bool {
    use std::{process::Stdio, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    tokio::time::timeout(Duration::from_secs(100), async {
        let Ok(mut child) = tokio::process::Command::new("/usr/bin/osascript")
            .args(["-l", "JavaScript", "-e", DIALOG])
            .env_clear()
            .env("LC_ALL", "C")
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        else {
            return false;
        };
        let Some(mut input) = child.stdin.take() else {
            return false;
        };
        if input.write_all(&body).await.is_err() {
            return false;
        }
        drop(input);
        let Some(output) = child.stdout.take() else {
            return false;
        };
        let mut result = Vec::new();
        if output.take(16).read_to_end(&mut result).await.is_err() {
            return false;
        }
        child.wait().await.is_ok_and(|status| status.success()) && approved_output(&result)
    })
    .await
    .unwrap_or(false)
}

pub(super) struct Review {
    offer: u32,
    task: Option<tokio::task::JoinHandle<bool>>,
}
impl Review {
    pub(super) fn start(offer: u32, start: &str, confirm: &WorkConfirmV1) -> Self {
        let body = dialog_body(start, confirm);
        Self {
            offer,
            task: Some(tokio::spawn(async move {
                match body {
                    Some(body) => native_review(body).await,
                    None => false,
                }
            })),
        }
    }
    pub(super) fn ready(&self) -> bool {
        self.task.as_ref().is_some_and(|task| task.is_finished())
    }
    pub(super) async fn finish(&mut self) -> Decision {
        let approve = match self.task.take() {
            Some(task) => task.await.unwrap_or(false),
            None => false,
        };
        Decision {
            offer: self.offer,
            approve,
        }
    }
}
impl Drop for Review {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[derive(Default)]
pub(super) struct Gate {
    next: u32,
    used: bool,
    pending: Option<(u32, WorkExecutionId, WorkStepId, WorkConfirmV1)>,
}

impl Gate {
    pub(super) fn eligible(confirm: &WorkConfirmV1) -> bool {
        confirm.site == "slack.com"
            && confirm.category == WorkConfirmCategoryV1::Communication
            && confirm.text.as_deref() == Some(MESSAGE)
            && confirm.page.is_some()
            && confirm.provenance.is_empty()
            && !confirm.run_option
            && confirm.decision.is_none()
    }

    /// A changed or replaced confirmation invalidates the old offer even when
    /// its human-readable text happens to match.
    pub(super) fn offer(
        &mut self,
        execution: WorkExecutionId,
        step: WorkStepId,
        confirm: &WorkConfirmV1,
    ) -> Option<(u32, bool)> {
        if self.used || !Self::eligible(confirm) {
            self.pending = None;
            return None;
        }
        if let Some((offer, old_execution, old_step, old)) = &self.pending {
            if *old_execution == execution && *old_step == step && old == confirm {
                return Some((*offer, false));
            }
        }
        self.pending = None;
        self.next = self.next.checked_add(1)?;
        self.pending = Some((self.next, execution, step, confirm.clone()));
        Some((self.next, true))
    }

    pub(super) fn clear(&mut self) {
        self.pending = None;
    }

    pub(super) fn decide(
        &mut self,
        decision: Decision,
        execution: WorkExecutionId,
        step: WorkStepId,
        confirm: &WorkConfirmV1,
    ) -> Option<bool> {
        let (offer, held_execution, held_step, held) = self.pending.as_ref()?;
        if self.used
            || decision.offer != *offer
            || execution != *held_execution
            || step != *held_step
            || held != confirm
            || !Self::eligible(confirm)
        {
            return None;
        }
        // Never replay after uncertain command acknowledgement.
        self.used = true;
        self.pending = None;
        Some(decision.approve)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn confirm() -> WorkConfirmV1 {
        WorkConfirmV1 {
            site: "slack.com".into(),
            category: WorkConfirmCategoryV1::Communication,
            headline: "Send the qualification message?".into(),
            action: "press Send".into(),
            text: Some(MESSAGE.into()),
            facts: vec![],
            page: Some(1.into()),
            provenance: vec![],
            run_option: false,
            decision: None,
        }
    }
    #[test]
    fn only_the_closed_native_approval_result_is_accepted() {
        assert!(approved_output(b"approved\n"));
        for bytes in [
            "approved",
            "declined\n",
            " approved\n",
            "approved\napproved\n",
            "true",
            "approve 1",
        ] {
            assert!(!approved_output(bytes.as_bytes()));
        }
        let mut c = confirm();
        c.headline = "Untrusted \"quote\"\\\n); throw 'inert';".into();
        let body = dialog_body("https://app.slack.com/client/T0/C0", &c).unwrap();
        let decoded: String = serde_json::from_slice(&body).unwrap();
        assert!(decoded.contains(&c.headline));
        assert!(decoded.contains(MESSAGE));
        assert!(!DIALOG.contains(&c.headline));
        c.facts = vec![WorkConfirmFactV1 {
            label: "x".into(),
            value: "x".repeat(301),
        }];
        assert!(dialog_body("https://app.slack.com/client/T0/C0", &c).is_none());
    }
    #[test]
    fn approvals_are_exact_single_use_and_never_cover_other_effects() {
        assert_eq!(
            zephium_app::work_sites::site_of("https://app.slack.com/client/T0/C0").as_deref(),
            Some("slack.com")
        );
        let mut gate = Gate::default();
        let original = confirm();
        let yes = Decision {
            offer: 1,
            approve: true,
        };
        assert_eq!(gate.decide(yes, 1.into(), 2.into(), &original), None);
        assert_eq!(gate.offer(1.into(), 2.into(), &original), Some((1, true)));
        assert_eq!(gate.offer(1.into(), 2.into(), &original), Some((1, false)));
        assert_eq!(gate.decide(yes, 2.into(), 2.into(), &original), None);
        let mut changed = original.clone();
        changed.facts.push(WorkConfirmFactV1 {
            label: "Channel".into(),
            value: "other".into(),
        });
        assert_eq!(gate.decide(yes, 1.into(), 2.into(), &changed), None);
        assert_eq!(gate.offer(1.into(), 2.into(), &changed), Some((2, true)));
        assert_eq!(gate.decide(yes, 1.into(), 2.into(), &changed), None);
        assert_eq!(
            gate.decide(
                Decision {
                    offer: 2,
                    approve: true
                },
                1.into(),
                2.into(),
                &changed
            ),
            Some(true)
        );
        assert_eq!(gate.offer(1.into(), 3.into(), &original), None);
        for category in [
            WorkConfirmCategoryV1::Purchase,
            WorkConfirmCategoryV1::Destructive,
            WorkConfirmCategoryV1::Save,
            WorkConfirmCategoryV1::Edit,
            WorkConfirmCategoryV1::Type,
        ] {
            let mut c = original.clone();
            c.category = category;
            assert!(!Gate::eligible(&c));
        }
        let mut c = original.clone();
        c.text = Some(format!("{MESSAGE} extra"));
        assert!(!Gate::eligible(&c));
        c = original.clone();
        c.site = "slack.com.evil.test".into();
        assert!(!Gate::eligible(&c));
        c = original;
        c.page = None;
        assert!(!Gate::eligible(&c));
    }
    #[test]
    fn vanished_offers_and_declines_cannot_be_replayed() {
        let c = confirm();
        let mut gate = Gate::default();
        gate.offer(1.into(), 2.into(), &c);
        gate.clear();
        assert_eq!(
            gate.decide(
                Decision {
                    offer: 1,
                    approve: true
                },
                1.into(),
                2.into(),
                &c
            ),
            None
        );
        assert_eq!(gate.offer(1.into(), 2.into(), &c), Some((2, true)));
        assert_eq!(
            gate.decide(
                Decision {
                    offer: 2,
                    approve: false
                },
                1.into(),
                2.into(),
                &c
            ),
            Some(false)
        );
        assert_eq!(gate.offer(1.into(), 2.into(), &c), None);
    }
    #[tokio::test]
    async fn leaving_the_account_flow_cancels_a_pending_review() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        struct Finished(Arc<AtomicBool>);
        impl Drop for Finished {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let finished = Arc::new(AtomicBool::new(false));
        let observed = finished.clone();
        let (ready, started) = tokio::sync::oneshot::channel();
        let review = Review {
            offer: 1,
            task: Some(tokio::spawn(async move {
                let _finished = Finished(observed);
                let _ = ready.send(());
                std::future::pending::<bool>().await
            })),
        };
        started.await.unwrap();
        assert!(!review.ready());
        drop(review);
        tokio::time::timeout(std::time::Duration::from_millis(100), async {
            while !finished.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
