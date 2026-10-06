//! The providers an engine knows, keyed by [`ProviderId`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::{Locator, Machine, MachineId, Provider, ProviderId, PwError, Result};
use crate::exec::Exec;

pub struct Providers {
    list: Vec<Arc<dyn Provider>>,
    /// Machines per provider, looked up once (labels for views).
    machines: Mutex<HashMap<ProviderId, Vec<Machine>>>,
}

impl Providers {
    /// The first provider that can create agents is where new agents go.
    pub fn new(list: Vec<Arc<dyn Provider>>) -> Providers {
        Providers { list, machines: Mutex::default() }
    }

    pub fn all(&self) -> &[Arc<dyn Provider>] {
        &self.list
    }

    pub fn get(&self, id: &ProviderId) -> Result<Arc<dyn Provider>> {
        self.list
            .iter()
            .find(|p| p.id() == id)
            .cloned()
            .ok_or_else(|| PwError::unreachable(format!("no provider \"{id}\"")))
    }

    pub fn for_locator(&self, loc: &Locator) -> Result<Arc<dyn Provider>> {
        self.get(&loc.provider)
    }

    /// Where new agents are created: the first provider that can, on its
    /// first machine.
    pub fn default_target(&self) -> Result<(Arc<dyn Provider>, Machine)> {
        let p = self
            .list
            .iter()
            .find(|p| p.caps().create)
            .cloned()
            .ok_or_else(|| PwError::unsupported("no provider can start agents"))?;
        let m = self.machines(&p)?.into_iter().next().ok_or_else(|| PwError::unreachable(format!("{} has no machines", p.id())))?;
        Ok((p, m))
    }

    /// A provider's machines (cached after the first successful answer).
    pub fn machines(&self, p: &Arc<dyn Provider>) -> Result<Vec<Machine>> {
        if let Some(m) = self.machines.lock().unwrap_or_else(|e| e.into_inner()).get(p.id()) {
            return Ok(m.clone());
        }
        let m = p.machines()?;
        self.machines.lock().unwrap_or_else(|e| e.into_inner()).insert(p.id().clone(), m.clone());
        Ok(m)
    }

    /// How a machine is shown; its id when the provider doesn't list it.
    pub fn machine_label(&self, loc: &Locator) -> String {
        self.for_locator(loc)
            .and_then(|p| self.machines(&p))
            .ok()
            .and_then(|ms| ms.into_iter().find(|m| m.id == loc.machine))
            .map_or_else(|| loc.machine.to_string(), |m| m.label)
    }

    /// Commands and files on the machine of `loc`.
    pub fn exec(&self, loc: &Locator) -> Result<Arc<dyn Exec>> {
        self.for_locator(loc)?.exec(&loc.machine)
    }

    pub fn exec_on(&self, provider: &ProviderId, machine: &MachineId) -> Result<Arc<dyn Exec>> {
        self.get(provider)?.exec(machine)
    }
}
