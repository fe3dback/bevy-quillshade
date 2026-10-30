//! The active domain (design §14.2, §14.4): the overlay rule of project tool tabs, each domain's last active tab,
//! switching domains, which tabs are greyed, the successor of a closed active tab and the user config's part of it.

use crate::config::ProjectTabs;
use crate::domain::TOOL_KEY_PREFIX;
use crate::settings::SETTINGS_TOOL;
use crate::tab::TabKey;
use project_core::DomainId;

use super::editor::Editor;

impl Editor {
    /// The overlay rule (design §14.2), for tab `i` being activated: a real domain's tab makes its domain active, a
    /// project tool's tab leaves the active domain as it is; both become their domain's last active tab.
    pub(crate) fn follow_tab_domain(&mut self, i: usize) {
        let Some(tab) = self.tabs.get(i) else { return };
        let (domain, key) = (tab.domain, tab.key());
        if domain != DomainId::PROJECT && self.active_domain != Some(domain) {
            self.active_domain = Some(domain);
            self.ui.browser += 1;
        }
        if let Some(key) = key {
            self.last_tabs.insert(domain, key);
        }
    }

    /// Whether tab `i` is shown greyed: it belongs to a real domain other than the active one (design §14.3). Project
    /// tool tabs never are.
    pub fn is_greyed(&self, i: usize) -> bool {
        self.tabs.get(i).is_some_and(|t| t.domain != DomainId::PROJECT && Some(t.domain) != self.active_domain)
    }

    /// `Action::SwitchDomain` (design §14.3): makes `id` the active domain and activates its last active tab if that
    /// is open; else the project domain opens Project Settings (design §14.2), and any other leaves no tab active (the
    /// editor content shows the empty hint). The active domain, an unknown one or no project: nothing happens.
    pub fn switch_domain(&mut self, id: DomainId) {
        if self.project.is_none() || self.active_domain == Some(id) || self.domain_list.get(id).is_none() {
            return;
        }
        self.active_domain = Some(id);
        match self.last_tabs.get(&id).and_then(|k| self.key_index(k)) {
            Some(i) => self.activate(i),
            None if id == DomainId::PROJECT && self.domain_list.tool(id, SETTINGS_TOOL).is_some() => {
                self.open_tool(id, SETTINGS_TOOL, None);
            }
            None => self.active = None,
        }
        self.touch_all();
    }

    /// The tab to activate when the active tab `a` closes (`close` marks the closing tabs), as an index among the
    /// tabs that stay: the nearest staying tab of the active domain or a project tool, looking right first, then left
    /// (design §14.2); `None` when there is none, so closing never jumps to another domain.
    pub(crate) fn close_successor(&self, a: usize, close: &[bool]) -> Option<usize> {
        let fits = |j: &usize| {
            let domain = self.tabs[*j].domain;
            !close[*j] && (domain == DomainId::PROJECT || Some(domain) == self.active_domain)
        };
        let next = (a + 1..self.tabs.len()).find(fits).or_else(|| (0..a).rev().find(fits))?;
        Some((0..next).filter(|j| !close[*j]).count())
    }

    /// Reads the active domain and the last tabs of a project being opened from its user config entry, after its tabs
    /// were restored: the saved domain if it is still registered, else the first real domain; saved last tabs of
    /// registered real domains replace the ones restoring recorded (tool keys are skipped). A restored active tab
    /// that would be greyed gives way to the active domain's last tab, or to none (design §14.4).
    pub(crate) fn restore_domain(&mut self, saved: &ProjectTabs) {
        let list = self.domain_list.clone();
        let real = |id: &str| list.real_ids().find(|d| d.as_str() == id);
        for (id, key) in &saved.last_tabs {
            if let Some(domain) = real(id).filter(|_| !key.starts_with(TOOL_KEY_PREFIX)) {
                self.last_tabs.insert(domain, TabKey::new(domain, key.clone()));
            }
        }
        let saved_domain = saved.active_domain.as_deref().and_then(|id| list.ids().find(|d| d.as_str() == id));
        self.active_domain = saved_domain.or_else(|| list.real_ids().next());
        match self.active {
            Some(a) if self.is_greyed(a) => {
                self.active = self.active_domain.and_then(|d| self.last_tabs.get(&d)).and_then(|k| self.key_index(k));
            }
            Some(a) => self.follow_tab_domain(a),
            None => {}
        }
    }

    /// The user config's part of the active domain (design §14.4): its id and each real domain's last tab key; tool
    /// keys are never written.
    pub(crate) fn saved_domain_state(&self) -> (Option<String>, std::collections::BTreeMap<String, String>) {
        let last =
            self.last_tabs.iter().filter(|(d, k)| **d != DomainId::PROJECT && !k.key.starts_with(TOOL_KEY_PREFIX));
        let last = last.map(|(d, k)| (d.as_str().to_string(), k.key.clone())).collect();
        (self.active_domain.map(|d| d.as_str().to_string()), last)
    }
}
