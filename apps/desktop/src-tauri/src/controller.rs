use std::collections::{BTreeMap, HashMap};

use romm_ipc::{ControllerBindings, ControllerButton};
use serde::Serialize;

pub const DUPLICATE_WINDOW_MS: u64 = 30;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ControllerActionKind {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
    Search,
    Context,
    PreviousTab,
    NextTab,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ControllerActionPhase {
    Pressed,
    Repeated,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MappingFamily {
    Xbox,
    PlayStation,
    Nintendo,
    Steam,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerInfo {
    pub id: String,
    pub guid: String,
    pub name: String,
    pub mapping_family: MappingFamily,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerAction {
    pub action: ControllerActionKind,
    pub phase: ControllerActionPhase,
    pub controller_id: String,
    pub controller_name: String,
    pub mapping_family: MappingFamily,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerStatusChange {
    Initialized,
    Connected,
    Disconnected,
    ActiveChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerStatus {
    pub controllers: Vec<ControllerInfo>,
    pub active_controller_id: Option<String>,
    pub change: ControllerStatusChange,
    pub changed_controller: Option<ControllerInfo>,
    pub timestamp_ms: u64,
}

#[derive(Debug, Default)]
struct ControllerState {
    info: Option<ControllerInfo>,
    horizontal: i8,
    vertical: i8,
    activity_sequence: u64,
    held_directions: BTreeMap<ControllerActionKind, u64>,
}

#[derive(Debug, Default)]
pub struct ControllerPipeline {
    controllers: BTreeMap<String, ControllerState>,
    active_controller_id: Option<String>,
    sequence: u64,
    last_emitted_at: HashMap<(ControllerActionKind, ControllerActionPhase), u64>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ProcessedInput {
    pub actions: Vec<ControllerAction>,
    pub active_changed: bool,
}

impl ControllerPipeline {
    pub fn controller_info(&self, controller_id: &str) -> Option<ControllerInfo> {
        self.controllers.get(controller_id)?.info.clone()
    }

    pub fn connect(&mut self, info: ControllerInfo) {
        self.sequence = self.sequence.saturating_add(1);
        let state = self.controllers.entry(info.id.clone()).or_default();
        state.info = Some(info.clone());
        state.horizontal = 0;
        state.vertical = 0;
        state.held_directions.clear();
        state.activity_sequence = self.sequence;
        if self.active_controller_id.is_none() {
            self.active_controller_id = Some(info.id);
        }
    }

    pub fn disconnect(&mut self, controller_id: &str) -> Option<ControllerInfo> {
        let removed = self.controllers.remove(controller_id)?.info;
        if self.active_controller_id.as_deref() == Some(controller_id) {
            self.active_controller_id = self
                .controllers
                .iter()
                .max_by_key(|(_, state)| state.activity_sequence)
                .map(|(id, _)| id.clone());
        }
        removed
    }

    pub fn status(
        &self,
        change: ControllerStatusChange,
        changed_controller: Option<ControllerInfo>,
        timestamp_ms: u64,
    ) -> ControllerStatus {
        ControllerStatus {
            controllers: self
                .controllers
                .values()
                .filter_map(|state| state.info.clone())
                .collect(),
            active_controller_id: self.active_controller_id.clone(),
            change,
            changed_controller,
            timestamp_ms,
        }
    }

    pub fn button(
        &mut self,
        controller_id: &str,
        action: ControllerActionKind,
        phase: ControllerActionPhase,
        timestamp_ms: u64,
        initial_repeat_delay_ms: u16,
    ) -> ProcessedInput {
        let active_changed = self.mark_active(controller_id);
        let actions = self
            .emit(
                controller_id,
                action,
                phase,
                timestamp_ms,
                initial_repeat_delay_ms,
            )
            .into_iter()
            .collect();
        ProcessedInput {
            actions,
            active_changed,
        }
    }

    pub fn horizontal_axis(
        &mut self,
        controller_id: &str,
        next: i8,
        timestamp_ms: u64,
        initial_repeat_delay_ms: u16,
    ) -> ProcessedInput {
        self.axis(
            controller_id,
            next,
            timestamp_ms,
            initial_repeat_delay_ms,
            true,
        )
    }

    pub fn vertical_axis(
        &mut self,
        controller_id: &str,
        next: i8,
        timestamp_ms: u64,
        initial_repeat_delay_ms: u16,
    ) -> ProcessedInput {
        self.axis(
            controller_id,
            next,
            timestamp_ms,
            initial_repeat_delay_ms,
            false,
        )
    }

    fn axis(
        &mut self,
        controller_id: &str,
        next: i8,
        timestamp_ms: u64,
        initial_repeat_delay_ms: u16,
        horizontal: bool,
    ) -> ProcessedInput {
        let Some(state) = self.controllers.get_mut(controller_id) else {
            return ProcessedInput::default();
        };
        let current = if horizontal {
            &mut state.horizontal
        } else {
            &mut state.vertical
        };
        if *current == next {
            return ProcessedInput::default();
        }

        let previous = *current;
        // Require the stick to return to neutral before accepting the opposite direction.
        *current = if previous != 0 && next == -previous {
            0
        } else {
            next
        };
        let accepted = *current;
        let mut requested = Vec::with_capacity(2);
        if let Some(action) = axis_action(horizontal, previous) {
            requested.push((action, ControllerActionPhase::Released));
        }
        if let Some(action) = axis_action(horizontal, accepted) {
            requested.push((action, ControllerActionPhase::Pressed));
        }

        let active_changed = if requested.is_empty() {
            false
        } else {
            self.mark_active(controller_id)
        };
        let actions = requested
            .into_iter()
            .filter_map(|(action, phase)| {
                self.emit(
                    controller_id,
                    action,
                    phase,
                    timestamp_ms,
                    initial_repeat_delay_ms,
                )
            })
            .collect();
        ProcessedInput {
            actions,
            active_changed,
        }
    }

    fn mark_active(&mut self, controller_id: &str) -> bool {
        let Some(state) = self.controllers.get_mut(controller_id) else {
            return false;
        };
        self.sequence = self.sequence.saturating_add(1);
        state.activity_sequence = self.sequence;
        if self.active_controller_id.as_deref() == Some(controller_id) {
            false
        } else {
            self.active_controller_id = Some(controller_id.to_owned());
            true
        }
    }

    fn emit(
        &mut self,
        controller_id: &str,
        action: ControllerActionKind,
        phase: ControllerActionPhase,
        timestamp_ms: u64,
        initial_repeat_delay_ms: u16,
    ) -> Option<ControllerAction> {
        let state = self.controllers.get_mut(controller_id)?;
        if is_directional(action) {
            match phase {
                ControllerActionPhase::Pressed => {
                    state.held_directions.insert(
                        action,
                        timestamp_ms.saturating_add(u64::from(initial_repeat_delay_ms)),
                    );
                }
                ControllerActionPhase::Released => {
                    state.held_directions.remove(&action);
                }
                ControllerActionPhase::Repeated => {}
            }
        }
        let info = state.info.as_ref()?.clone();
        let duplicate = self
            .last_emitted_at
            .get(&(action, phase))
            .is_some_and(|previous| timestamp_ms.saturating_sub(*previous) <= DUPLICATE_WINDOW_MS);
        if duplicate {
            return None;
        }
        self.last_emitted_at.insert((action, phase), timestamp_ms);
        Some(ControllerAction {
            action,
            phase,
            controller_id: info.id,
            controller_name: info.name,
            mapping_family: info.mapping_family,
            timestamp_ms,
        })
    }

    pub fn poll_repeats(
        &mut self,
        timestamp_ms: u64,
        repeat_interval_ms: u16,
    ) -> Vec<ControllerAction> {
        let interval = u64::from(repeat_interval_ms);
        let mut due = Vec::new();
        for (controller_id, state) in &mut self.controllers {
            for (action, next_repeat_at) in &mut state.held_directions {
                if timestamp_ms >= *next_repeat_at {
                    due.push((controller_id.clone(), *action));
                    *next_repeat_at = timestamp_ms.saturating_add(interval);
                }
            }
        }
        due.into_iter()
            .filter_map(|(controller_id, action)| {
                self.emit(
                    &controller_id,
                    action,
                    ControllerActionPhase::Repeated,
                    timestamp_ms,
                    0,
                )
            })
            .collect()
    }
}

pub fn action_for_button(
    button: ControllerButton,
    bindings: &ControllerBindings,
) -> Option<ControllerActionKind> {
    [
        (ControllerActionKind::Confirm, bindings.confirm),
        (ControllerActionKind::Back, bindings.back),
        (ControllerActionKind::Context, bindings.context),
        (ControllerActionKind::Search, bindings.search),
        (ControllerActionKind::PreviousTab, bindings.previous_tab),
        (ControllerActionKind::NextTab, bindings.next_tab),
    ]
    .into_iter()
    .find_map(|(action, assigned)| (assigned == Some(button)).then_some(action))
}

pub fn axis_direction(value: f32, dead_zone_percent: u8) -> i8 {
    let dead_zone = f32::from(dead_zone_percent) / 100.0;
    if value > dead_zone {
        1
    } else if value < -dead_zone {
        -1
    } else {
        0
    }
}

fn is_directional(action: ControllerActionKind) -> bool {
    matches!(
        action,
        ControllerActionKind::Up
            | ControllerActionKind::Down
            | ControllerActionKind::Left
            | ControllerActionKind::Right
    )
}

pub fn mapping_family(vendor_id: Option<u16>, name: &str) -> MappingFamily {
    match vendor_id {
        Some(0x045e) => return MappingFamily::Xbox,
        Some(0x054c) => return MappingFamily::PlayStation,
        Some(0x057e) => return MappingFamily::Nintendo,
        Some(0x28de) => return MappingFamily::Steam,
        _ => {}
    }

    let name = name.to_ascii_lowercase();
    if name.contains("xbox") || name.contains("xinput") {
        MappingFamily::Xbox
    } else if [
        "dualshock",
        "dualsense",
        "playstation",
        "wireless controller",
    ]
    .iter()
    .any(|needle| name.contains(needle))
    {
        MappingFamily::PlayStation
    } else if name.contains("nintendo") || name.contains("switch") || name.contains("joy-con") {
        MappingFamily::Nintendo
    } else if name.contains("steam") {
        MappingFamily::Steam
    } else {
        MappingFamily::Generic
    }
}

fn axis_action(horizontal: bool, direction: i8) -> Option<ControllerActionKind> {
    match (horizontal, direction) {
        (true, -1) => Some(ControllerActionKind::Left),
        (true, 1) => Some(ControllerActionKind::Right),
        (false, -1) => Some(ControllerActionKind::Down),
        (false, 1) => Some(ControllerActionKind::Up),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, name: &str) -> ControllerInfo {
        ControllerInfo {
            id: id.to_owned(),
            guid: format!("{id:0>32}"),
            name: name.to_owned(),
            mapping_family: mapping_family(None, name),
        }
    }

    #[test]
    fn most_recent_controller_becomes_active_and_disconnect_promotes_a_fallback() {
        let mut pipeline = ControllerPipeline::default();
        pipeline.connect(info("one", "Xbox Controller"));
        pipeline.connect(info("two", "DualSense Wireless Controller"));
        assert_eq!(pipeline.active_controller_id.as_deref(), Some("one"));

        let input = pipeline.button(
            "two",
            ControllerActionKind::Confirm,
            ControllerActionPhase::Pressed,
            100,
            350,
        );
        assert!(input.active_changed);
        assert_eq!(pipeline.active_controller_id.as_deref(), Some("two"));

        let removed = pipeline.disconnect("two").expect("controller should exist");
        assert_eq!(removed.name, "DualSense Wireless Controller");
        assert_eq!(pipeline.active_controller_id.as_deref(), Some("one"));
    }

    #[test]
    fn axis_state_is_independent_per_controller_and_requires_neutral_for_opposites() {
        let mut pipeline = ControllerPipeline::default();
        pipeline.connect(info("one", "One"));
        pipeline.connect(info("two", "Two"));

        assert_eq!(
            pipeline.horizontal_axis("one", 1, 100, 350).actions.len(),
            1
        );
        assert_eq!(
            pipeline.horizontal_axis("two", 1, 200, 350).actions.len(),
            1
        );
        let opposite = pipeline.horizontal_axis("one", -1, 300, 350);
        assert_eq!(opposite.actions.len(), 1);
        assert_eq!(opposite.actions[0].phase, ControllerActionPhase::Released);
        assert!(
            pipeline
                .horizontal_axis("one", 0, 400, 350)
                .actions
                .is_empty()
        );
        let accepted = pipeline.horizontal_axis("one", -1, 500, 350);
        assert_eq!(accepted.actions[0].action, ControllerActionKind::Left);
    }

    #[test]
    fn duplicate_actions_inside_thirty_milliseconds_are_suppressed() {
        let mut pipeline = ControllerPipeline::default();
        pipeline.connect(info("one", "Xbox Controller"));

        assert_eq!(
            pipeline
                .button(
                    "one",
                    ControllerActionKind::Down,
                    ControllerActionPhase::Pressed,
                    1_000,
                    350,
                )
                .actions
                .len(),
            1
        );
        assert!(
            pipeline
                .button(
                    "one",
                    ControllerActionKind::Down,
                    ControllerActionPhase::Pressed,
                    1_025,
                    350,
                )
                .actions
                .is_empty()
        );
        assert_eq!(
            pipeline
                .button(
                    "one",
                    ControllerActionKind::Down,
                    ControllerActionPhase::Pressed,
                    1_031,
                    350,
                )
                .actions
                .len(),
            1
        );
    }

    #[test]
    fn mapping_family_uses_vendor_then_safe_name_fallbacks() {
        assert_eq!(
            mapping_family(Some(0x054c), "Unknown"),
            MappingFamily::PlayStation
        );
        assert_eq!(
            mapping_family(None, "Xbox Wireless Controller"),
            MappingFamily::Xbox
        );
        assert_eq!(mapping_family(None, "Steam Deck"), MappingFamily::Steam);
        assert_eq!(mapping_family(None, "Mystery Pad"), MappingFamily::Generic);
    }

    #[test]
    fn normalized_events_serialize_with_the_frontend_contract() {
        let action = ControllerAction {
            action: ControllerActionKind::PreviousTab,
            phase: ControllerActionPhase::Repeated,
            controller_id: "controller-1".to_owned(),
            controller_name: "DualSense".to_owned(),
            mapping_family: MappingFamily::PlayStation,
            timestamp_ms: 1234,
        };
        let value = serde_json::to_value(action).expect("controller action should serialize");

        assert_eq!(value["action"], "previousTab");
        assert_eq!(value["phase"], "repeated");
        assert_eq!(value["controllerId"], "controller-1");
        assert_eq!(value["mappingFamily"], "play_station");
        assert_eq!(value["timestampMs"], 1234);
    }

    #[test]
    fn configured_buttons_map_to_exactly_one_action() {
        let bindings = ControllerBindings {
            confirm: Some(ControllerButton::East),
            back: Some(ControllerButton::South),
            ..ControllerBindings::default()
        };
        assert_eq!(
            action_for_button(ControllerButton::East, &bindings),
            Some(ControllerActionKind::Confirm)
        );
        assert_eq!(
            action_for_button(ControllerButton::South, &bindings),
            Some(ControllerActionKind::Back)
        );
    }

    #[test]
    fn held_directions_repeat_after_the_configured_delay_and_interval() {
        let mut pipeline = ControllerPipeline::default();
        pipeline.connect(info("one", "Xbox Controller"));
        let pressed = pipeline.button(
            "one",
            ControllerActionKind::Down,
            ControllerActionPhase::Pressed,
            1_000,
            350,
        );
        assert_eq!(pressed.actions.len(), 1);
        assert!(pipeline.poll_repeats(1_349, 100).is_empty());
        let first_repeat = pipeline.poll_repeats(1_350, 100);
        assert_eq!(first_repeat.len(), 1);
        assert_eq!(first_repeat[0].phase, ControllerActionPhase::Repeated);
        assert!(pipeline.poll_repeats(1_449, 100).is_empty());
        assert_eq!(pipeline.poll_repeats(1_450, 100).len(), 1);
        let _ = pipeline.button(
            "one",
            ControllerActionKind::Down,
            ControllerActionPhase::Released,
            1_451,
            350,
        );
        assert!(pipeline.poll_repeats(2_000, 100).is_empty());
    }

    #[test]
    fn dead_zone_uses_the_configured_percentage() {
        assert_eq!(axis_direction(0.24, 25), 0);
        assert_eq!(axis_direction(0.26, 25), 1);
        assert_eq!(axis_direction(-0.26, 25), -1);
        assert_eq!(axis_direction(0.26, 30), 0);
    }
}
