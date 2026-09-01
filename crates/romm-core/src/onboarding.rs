use std::collections::BTreeSet;

use romm_ipc::{
    ONBOARDING_STATE_VERSION, OnboardingNavigationAction, OnboardingState, OnboardingStep,
};

pub const ONBOARDING_STEPS: [OnboardingStep; 8] = [
    OnboardingStep::Server,
    OnboardingStep::Authentication,
    OnboardingStep::Permissions,
    OnboardingStep::Device,
    OnboardingStep::Detection,
    OnboardingStep::Mappings,
    OnboardingStep::Background,
    OnboardingStep::FirstRefresh,
];

pub fn validate_onboarding_state(state: &OnboardingState) -> Result<(), String> {
    if state.version != ONBOARDING_STATE_VERSION {
        return Err(format!(
            "Unsupported onboarding state version {}.",
            state.version
        ));
    }
    let completed: BTreeSet<_> = state.completed_steps.iter().copied().collect();
    if completed.len() != state.completed_steps.len() {
        return Err("Completed onboarding steps cannot contain duplicates.".to_owned());
    }
    if completed.contains(&OnboardingStep::Server) && state.server_origin.is_none() {
        return Err("A completed server step requires a normalized server origin.".to_owned());
    }
    if let Some(http_origin) = state.acknowledged_http_warning_origin.as_deref()
        && Some(http_origin) != state.server_origin.as_deref()
    {
        return Err("HTTP approval must belong to the selected server origin.".to_owned());
    }
    if state.selected_platform_ids.iter().any(|id| *id <= 0) {
        return Err("Selected RomM platform IDs must be positive.".to_owned());
    }
    if state
        .mapping_draft_ids
        .iter()
        .any(|id| id.trim().is_empty())
    {
        return Err("Mapping draft IDs cannot be empty.".to_owned());
    }
    if state
        .custom_path_drafts
        .iter()
        .any(|(key, path)| key.trim().is_empty() || path.trim().is_empty())
    {
        return Err("Custom path drafts require non-empty IDs and paths.".to_owned());
    }
    if state.highest_completed_step != highest_completed_step(state) {
        return Err("The highest completed onboarding step is inconsistent.".to_owned());
    }
    Ok(())
}

pub fn first_incomplete_step(state: &OnboardingState) -> OnboardingStep {
    ONBOARDING_STEPS
        .into_iter()
        .find(|step| !state.completed_steps.contains(step))
        .unwrap_or(OnboardingStep::FirstRefresh)
}

pub fn highest_completed_step(state: &OnboardingState) -> Option<OnboardingStep> {
    ONBOARDING_STEPS
        .into_iter()
        .rev()
        .find(|step| state.completed_steps.contains(step))
}

pub fn previous_onboarding_step(step: OnboardingStep) -> OnboardingStep {
    let index = ONBOARDING_STEPS
        .iter()
        .position(|candidate| *candidate == step)
        .unwrap_or_default();
    ONBOARDING_STEPS[index.saturating_sub(1)]
}

pub fn navigate_onboarding(
    state: &OnboardingState,
    action: OnboardingNavigationAction,
) -> OnboardingState {
    let mut next = state.clone();
    match action {
        OnboardingNavigationAction::Back => {
            next.cancelled = false;
            next.current_step = previous_onboarding_step(next.current_step);
        }
        OnboardingNavigationAction::Cancel => next.cancelled = true,
        OnboardingNavigationAction::Resume => {
            next.cancelled = false;
            next.current_step = first_incomplete_step(&next);
        }
    }
    next
}

pub fn restore_onboarding_state(state: &OnboardingState) -> Result<OnboardingState, String> {
    validate_onboarding_state(state)?;
    let mut restored = state.clone();
    if !restored.cancelled {
        restored.current_step = first_incomplete_step(&restored);
    }
    Ok(restored)
}

pub fn complete_onboarding_step(
    state: &OnboardingState,
    step: OnboardingStep,
) -> Result<OnboardingState, String> {
    let expected = first_incomplete_step(state);
    if step != expected && !state.completed_steps.contains(&step) {
        return Err(format!(
            "Cannot complete {step:?} before the current {expected:?} step."
        ));
    }
    let mut next = state.clone();
    if !next.completed_steps.contains(&step) {
        next.completed_steps.push(step);
        next.completed_steps.sort();
    }
    next.highest_completed_step = highest_completed_step(&next);
    next.current_step = first_incomplete_step(&next);
    next.cancelled = false;
    Ok(next)
}

pub fn select_onboarding_server(
    state: &OnboardingState,
    server_origin: &str,
    http_approved: bool,
) -> Result<OnboardingState, String> {
    let origin = server_origin.trim();
    if origin.is_empty() {
        return Err("The onboarding server origin cannot be empty.".to_owned());
    }
    let mut next = state.clone();
    let changed = next
        .server_origin
        .as_deref()
        .is_some_and(|saved| saved != origin);
    if changed {
        let invalidated = [
            OnboardingStep::Authentication,
            OnboardingStep::Permissions,
            OnboardingStep::Device,
            OnboardingStep::Mappings,
            OnboardingStep::FirstRefresh,
        ];
        next.completed_steps
            .retain(|step| !invalidated.contains(step));
        next.selected_platform_ids.clear();
        next.mapping_draft_ids.clear();
        // Custom path drafts, local detection, and explicit background choice are local data and
        // remain available for deliberate reuse on the replacement server.
    }
    next.server_origin = Some(origin.to_owned());
    next.acknowledged_http_warning_origin = http_approved.then(|| origin.to_owned());
    if !next.completed_steps.contains(&OnboardingStep::Server) {
        next.completed_steps.push(OnboardingStep::Server);
        next.completed_steps.sort();
    }
    next.highest_completed_step = highest_completed_step(&next);
    next.current_step = first_incomplete_step(&next);
    next.cancelled = false;
    validate_onboarding_state(&next)?;
    Ok(next)
}

pub fn invalidate_authenticated_onboarding(state: &OnboardingState) -> OnboardingState {
    let mut next = state.clone();
    next.completed_steps.retain(|step| {
        !matches!(
            step,
            OnboardingStep::Authentication
                | OnboardingStep::Permissions
                | OnboardingStep::FirstRefresh
        )
    });
    next.highest_completed_step = highest_completed_step(&next);
    next.current_step = first_incomplete_step(&next);
    next.cancelled = false;
    next
}

pub fn invalidate_device_onboarding(state: &OnboardingState) -> OnboardingState {
    let mut next = state.clone();
    next.completed_steps
        .retain(|step| !matches!(step, OnboardingStep::Device | OnboardingStep::FirstRefresh));
    next.highest_completed_step = highest_completed_step(&next);
    next.current_step = first_incomplete_step(&next);
    next.cancelled = false;
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_resumes_at_server() {
        let state = OnboardingState::default();
        assert_eq!(first_incomplete_step(&state), OnboardingStep::Server);
        assert_eq!(
            navigate_onboarding(&state, OnboardingNavigationAction::Resume),
            state
        );
        assert!(validate_onboarding_state(&state).is_ok());
    }

    #[test]
    fn completion_is_ordered_and_restores_the_first_gap() {
        let server = select_onboarding_server(
            &OnboardingState::default(),
            "https://romm.example.test",
            false,
        )
        .expect("server should save");
        assert_eq!(server.current_step, OnboardingStep::Authentication);
        assert!(complete_onboarding_step(&server, OnboardingStep::Device).is_err());
        let authentication = complete_onboarding_step(&server, OnboardingStep::Authentication)
            .expect("authentication should complete");
        assert_eq!(authentication.current_step, OnboardingStep::Permissions);
        assert_eq!(
            authentication.highest_completed_step,
            Some(OnboardingStep::Authentication)
        );
    }

    #[test]
    fn back_and_cancel_never_discard_valid_progress() {
        let server = select_onboarding_server(
            &OnboardingState::default(),
            "https://romm.example.test",
            false,
        )
        .expect("server should save");
        let backed = navigate_onboarding(&server, OnboardingNavigationAction::Back);
        assert_eq!(backed.current_step, OnboardingStep::Server);
        assert_eq!(backed.completed_steps, server.completed_steps);
        let cancelled = navigate_onboarding(&backed, OnboardingNavigationAction::Cancel);
        assert!(cancelled.cancelled);
        let resumed = navigate_onboarding(&cancelled, OnboardingNavigationAction::Resume);
        assert!(!resumed.cancelled);
        assert_eq!(resumed.current_step, OnboardingStep::Authentication);
    }

    #[test]
    fn reopening_resumes_the_first_gap_but_preserves_an_explicit_pause() {
        let server = select_onboarding_server(
            &OnboardingState::default(),
            "https://romm.example.test",
            false,
        )
        .expect("server should save");
        let backed = navigate_onboarding(&server, OnboardingNavigationAction::Back);
        assert_eq!(backed.current_step, OnboardingStep::Server);
        assert_eq!(
            restore_onboarding_state(&backed)
                .expect("state should restore")
                .current_step,
            OnboardingStep::Authentication
        );
        let cancelled = navigate_onboarding(&backed, OnboardingNavigationAction::Cancel);
        assert_eq!(
            restore_onboarding_state(&cancelled)
                .expect("cancelled state should restore")
                .current_step,
            OnboardingStep::Server
        );
    }

    #[test]
    fn every_persisted_wizard_boundary_resumes_without_repeating_completed_steps() {
        for completed_count in 0..=ONBOARDING_STEPS.len() {
            let completed_steps = ONBOARDING_STEPS[..completed_count].to_vec();
            let expected = ONBOARDING_STEPS
                .get(completed_count)
                .copied()
                .unwrap_or(OnboardingStep::FirstRefresh);
            let state = OnboardingState {
                current_step: OnboardingStep::Server,
                highest_completed_step: completed_steps.last().copied(),
                completed_steps: completed_steps.clone(),
                server_origin: (completed_count > 0)
                    .then(|| "https://romm.example.test".to_owned()),
                background_enabled: (completed_count > 6).then_some(false),
                ..OnboardingState::default()
            };

            let restored = restore_onboarding_state(&state).unwrap_or_else(|error| {
                panic!("boundary {completed_count} should restore: {error}")
            });
            assert_eq!(restored.current_step, expected);
            assert_eq!(restored.completed_steps, completed_steps);
        }
    }

    #[test]
    fn changing_servers_invalidates_only_server_dependent_progress() {
        let mut state = OnboardingState {
            version: ONBOARDING_STATE_VERSION,
            current_step: OnboardingStep::FirstRefresh,
            highest_completed_step: Some(OnboardingStep::Background),
            completed_steps: ONBOARDING_STEPS[..7].to_vec(),
            server_origin: Some("https://old.example.test".to_owned()),
            acknowledged_http_warning_origin: None,
            selected_platform_ids: vec![12, 14],
            mapping_draft_ids: vec!["server-preset".to_owned()],
            custom_path_drafts: [("nes".to_owned(), "D:\\ROMs\\NES".to_owned())]
                .into_iter()
                .collect(),
            background_enabled: Some(true),
            cancelled: false,
        };
        state.highest_completed_step = highest_completed_step(&state);
        let changed = select_onboarding_server(&state, "https://new.example.test", false)
            .expect("replacement server should save");
        assert!(changed.completed_steps.contains(&OnboardingStep::Server));
        assert!(changed.completed_steps.contains(&OnboardingStep::Detection));
        assert!(
            changed
                .completed_steps
                .contains(&OnboardingStep::Background)
        );
        assert!(
            !changed
                .completed_steps
                .contains(&OnboardingStep::Authentication)
        );
        assert!(!changed.completed_steps.contains(&OnboardingStep::Mappings));
        assert!(changed.selected_platform_ids.is_empty());
        assert!(changed.mapping_draft_ids.is_empty());
        assert_eq!(changed.custom_path_drafts, state.custom_path_drafts);
        assert_eq!(changed.background_enabled, Some(true));
        assert_eq!(changed.current_step, OnboardingStep::Authentication);
    }

    #[test]
    fn http_approval_is_scoped_to_the_selected_origin() {
        let approved =
            select_onboarding_server(&OnboardingState::default(), "http://romm.local:8080", true)
                .expect("HTTP approval should save");
        assert_eq!(
            approved.acknowledged_http_warning_origin.as_deref(),
            Some("http://romm.local:8080")
        );
        let changed = select_onboarding_server(&approved, "https://romm.example.test", false)
            .expect("HTTPS server should save");
        assert_eq!(changed.acknowledged_http_warning_origin, None);
    }

    #[test]
    fn logout_invalidates_session_steps_without_forgetting_the_device() {
        let state = OnboardingState {
            version: ONBOARDING_STATE_VERSION,
            current_step: OnboardingStep::Detection,
            highest_completed_step: Some(OnboardingStep::Device),
            completed_steps: ONBOARDING_STEPS[..4].to_vec(),
            server_origin: Some("https://romm.example.test".to_owned()),
            ..OnboardingState::default()
        };
        let invalidated = invalidate_authenticated_onboarding(&state);
        assert!(
            invalidated
                .completed_steps
                .contains(&OnboardingStep::Server)
        );
        assert!(
            invalidated
                .completed_steps
                .contains(&OnboardingStep::Device)
        );
        assert!(
            !invalidated
                .completed_steps
                .contains(&OnboardingStep::Authentication)
        );
        assert!(
            !invalidated
                .completed_steps
                .contains(&OnboardingStep::Permissions)
        );
        assert_eq!(invalidated.current_step, OnboardingStep::Authentication);
    }

    #[test]
    fn missing_remote_device_retains_local_work_but_requires_registration_again() {
        let state = OnboardingState {
            current_step: OnboardingStep::FirstRefresh,
            highest_completed_step: Some(OnboardingStep::Background),
            completed_steps: ONBOARDING_STEPS[..7].to_vec(),
            server_origin: Some("https://romm.example.test".to_owned()),
            ..OnboardingState::default()
        };

        let invalidated = invalidate_device_onboarding(&state);
        assert!(
            !invalidated
                .completed_steps
                .contains(&OnboardingStep::Device)
        );
        assert!(
            invalidated
                .completed_steps
                .contains(&OnboardingStep::Mappings)
        );
        assert_eq!(invalidated.current_step, OnboardingStep::Device);
    }
}
