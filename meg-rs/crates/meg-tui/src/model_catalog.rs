use crate::picker::{ActiveModal, ModelItem, PickerState};
use crate::WorkstationState;
use meg_core::providers::ProviderProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryStatus {
    Checking,
    Ready,
    Empty,
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct ProviderModels {
    pub profile: ProviderProfile,
    pub models: Vec<String>,
    pub status: DiscoveryStatus,
    pub message: String,
    pub request_id: String,
}
impl ProviderModels {
    pub fn label(&self) -> String {
        match self.status {
            DiscoveryStatus::Checking => "checking…".into(),
            DiscoveryStatus::Ready => format!("{} models", self.models.len()),
            DiscoveryStatus::Empty => "reachable · no models installed/advertised".into(),
            DiscoveryStatus::Unavailable if !self.models.is_empty() => {
                "unavailable · cached models".into()
            }
            DiscoveryStatus::Unavailable => "unavailable · check connection".into(),
        }
    }
}

pub fn picker(state: &WorkstationState) -> PickerState<ModelItem> {
    let mut entries: Vec<_> = state.provider_models.values().collect();
    entries.sort_by_key(|e| {
        (
            !meg_core::provider_catalog::is_local(&e.profile),
            e.profile.name.to_lowercase(),
        )
    });
    let mut items = Vec::new();
    for entry in &entries {
        for model in &entry.models {
            let mut item = ModelItem::new(model, &entry.profile.name, entry.label(), "");
            item.provider_id = entry.profile.id.clone();
            items.push(item.to_picker_item());
        }
    }
    if let Some(profile) = &state.provider {
        let models = if state.available_models.is_empty() {
            vec![state.model.clone()]
        } else {
            state.available_models.clone()
        };
        for model in models.into_iter().filter(|m| !m.is_empty()) {
            if !items
                .iter()
                .any(|it| it.value.id == model && it.value.provider_id == profile.id)
            {
                let mut item = ModelItem::new(model, &profile.name, "configured", "");
                item.provider_id = profile.id.clone();
                items.push(item.to_picker_item());
            }
        }
    }
    if state.provider.is_none() {
        for model in state.available_models.iter().filter(|m| !m.is_empty()) {
            items.push(ModelItem::new(model, "endpoint", "configured", "").to_picker_item());
        }
    }
    let mut p = PickerState::new("Models · connected providers & local servers".into(), items);
    p.footer = " Enter use · Ctrl+R refresh · Ctrl+N connect · Esc close".into();
    p.empty_message=" No models found. Ctrl+N connects a provider.\n Local models must be registered with a running server.".into();
    for entry in entries {
        p.empty_message
            .push_str(&format!("\n {}: {}", entry.profile.name, entry.label()));
    }
    if let Some(index) = p.items.iter().position(|it| {
        it.value.id == state.model
            && state
                .provider
                .as_ref()
                .is_some_and(|a| a.id == it.value.provider_id)
    }) {
        p.selected_index = index;
    }
    if let ActiveModal::ModelPicker(old) = &state.active_modal {
        p.search_query = old.search_query.clone();
        p.filter();
        if let Some(previous) = old.selected_item() {
            if let Some(pos) = p.filtered_indices.iter().position(|i| {
                p.items[*i].value.id == previous.value.id
                    && p.items[*i].value.provider_id == previous.value.provider_id
            }) {
                p.selected_index = pos;
            }
        }
    }
    p.adjust_scroll();
    p
}
