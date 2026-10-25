//! Model failures and costs in the panel's closed words.
use zephium_core::work::model::{
    HostedRefusal, WorkModelEntry, WorkModelError, WorkModelProvider, WorkModelUsage,
};
use zephium_ipc::page_assist::{PageAssistCostV1, PageAssistFaultV1, WorkModelFaultV1};
use zephium_page_assist::cost;

/// A failed model call as the person can act on it.
pub(crate) fn model_fault(error: WorkModelError, hosted: bool) -> PageAssistFaultV1 {
    let retry_after_ms = match error {
        WorkModelError::RateLimited { retry_after_ms }
        | WorkModelError::Hosted { retry_after_ms, .. } => {
            retry_after_ms.map(|ms| u32::try_from(ms).unwrap_or(u32::MAX))
        }
        _ => None,
    };
    let fault = match error {
        WorkModelError::MissingKey | WorkModelError::Unauthorized if hosted => {
            WorkModelFaultV1::SignedOut
        }
        WorkModelError::MissingKey => WorkModelFaultV1::MissingKey,
        WorkModelError::Unauthorized => WorkModelFaultV1::KeyRefused,
        WorkModelError::OverBudget if hosted => WorkModelFaultV1::HostedUsage,
        WorkModelError::OverBudget => WorkModelFaultV1::ProviderCredit,
        WorkModelError::Hosted { refusal, .. } => match refusal {
            HostedRefusal::PlanRequired => WorkModelFaultV1::PlanRequired,
            HostedRefusal::BillingReview => WorkModelFaultV1::BillingReview,
            HostedRefusal::OutOfCredits => WorkModelFaultV1::HostedUsage,
            HostedRefusal::DailyLimit => WorkModelFaultV1::DailyLimit,
            HostedRefusal::FreeUsagePaused => WorkModelFaultV1::FreeUsagePaused,
            HostedRefusal::ConfirmationLimited => WorkModelFaultV1::HostedBusy,
            HostedRefusal::Paused => WorkModelFaultV1::HostedPaused,
            HostedRefusal::AlreadySent
            | HostedRefusal::InvalidAudio
            | HostedRefusal::UnsupportedAudio
            | HostedRefusal::AudioTooLong => return PageAssistFaultV1::Unavailable,
        },
        WorkModelError::RateLimited { .. } => WorkModelFaultV1::RateLimited,
        WorkModelError::Overloaded => WorkModelFaultV1::ProviderDown,
        WorkModelError::Network => WorkModelFaultV1::Unreachable,
        WorkModelError::ContextTooLong => WorkModelFaultV1::TooLarge,
        WorkModelError::BadRequest | WorkModelError::Protocol | WorkModelError::Cancelled => {
            return PageAssistFaultV1::Unavailable
        }
    };
    PageAssistFaultV1::Model {
        fault,
        retry_after_ms,
    }
}

/// The provider as the cost line names it.
pub(crate) fn provider_name(provider: WorkModelProvider) -> &'static str {
    match provider {
        WorkModelProvider::OpenAi => "OpenAI",
        WorkModelProvider::Anthropic => "Anthropic",
        WorkModelProvider::Google => "Google",
        WorkModelProvider::DeepSeek => "DeepSeek",
        WorkModelProvider::OpenRouter => "OpenRouter",
        WorkModelProvider::XAi => "xAI",
        WorkModelProvider::Mistral => "Mistral",
        WorkModelProvider::Groq => "Groq",
        WorkModelProvider::Cerebras => "Cerebras",
        WorkModelProvider::Compatible => "your server",
        WorkModelProvider::Cloud => "Quasar",
    }
}

/// The cost line for a finished call. Hosted charges are confirmed later.
pub(crate) fn cost(entry: &WorkModelEntry, usage: &WorkModelUsage) -> PageAssistCostV1 {
    let model = entry.display_name.clone();
    match entry.model.provider {
        WorkModelProvider::Cloud => PageAssistCostV1::Hosted {
            model,
            credits: None,
        },
        WorkModelProvider::Compatible => PageAssistCostV1::Local { model },
        provider => {
            let micros = usage.cost_micros.or_else(|| {
                entry.price.as_ref().map(|price| {
                    let cached = usage.cached_input_tokens.min(usage.input_tokens);
                    cost::estimate_micros(
                        usage.input_tokens - cached,
                        usage.output_tokens,
                        price.input,
                        price.output,
                    )
                    .saturating_add(cost::estimate_micros(
                        cached,
                        0,
                        price.cached_input,
                        0,
                    ))
                })
            });
            PageAssistCostV1::Byok {
                model,
                provider: provider_name(provider).to_owned(),
                usd: micros.map(cost::usd),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faults_keep_the_retry_hint_and_hosting() {
        assert_eq!(
            model_fault(
                WorkModelError::RateLimited {
                    retry_after_ms: Some(7_000)
                },
                false
            ),
            PageAssistFaultV1::Model {
                fault: WorkModelFaultV1::RateLimited,
                retry_after_ms: Some(7_000)
            }
        );
        assert_eq!(
            model_fault(WorkModelError::MissingKey, true),
            PageAssistFaultV1::Model {
                fault: WorkModelFaultV1::SignedOut,
                retry_after_ms: None
            }
        );
        assert_eq!(
            model_fault(
                WorkModelError::Hosted {
                    refusal: HostedRefusal::DailyLimit,
                    retry_after_ms: Some(u64::MAX)
                },
                true
            ),
            PageAssistFaultV1::Model {
                fault: WorkModelFaultV1::DailyLimit,
                retry_after_ms: Some(u32::MAX)
            }
        );
        assert_eq!(
            model_fault(WorkModelError::Protocol, false),
            PageAssistFaultV1::Unavailable
        );
    }
}
