//! The Swimlanes board: machines top to bottom, states left to right. A chat drifts right as it cools,
//! inside its own machine's lane. Pure: the UI decides caps and folds, this decides membership and order.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::model::{Lamp, Session, Snapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Column {
    Need,
    Run,
    Warm,
    Cold,
    Quiet,
}

impl Column {
    pub const ALL: [Column; 5] = [Column::Need, Column::Run, Column::Warm, Column::Cold, Column::Quiet];
    pub fn of(l: Lamp) -> Column {
        match l {
            Lamp::Held | Lamp::Wait => Column::Need,
            Lamp::Run | Lamp::Stall => Column::Run,
            Lamp::Warm => Column::Warm,
            Lamp::Cold => Column::Cold,
            Lamp::Idle => Column::Quiet,
        }
    }
    pub fn index(self) -> usize {
        Column::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
    pub fn name(self) -> &'static str {
        match self {
            Column::Need => "Needs you",
            Column::Run => "Working",
            Column::Warm => "Warm",
            Column::Cold => "Cold",
            Column::Quiet => "Quiet",
        }
    }
    /// The lamp the column head shows.
    pub fn lamp(self) -> Lamp {
        match self {
            Column::Need => Lamp::Wait,
            Column::Run => Lamp::Run,
            Column::Warm => Lamp::Warm,
            Column::Cold => Lamp::Cold,
            Column::Quiet => Lamp::Idle,
        }
    }
    pub fn quiet(self) -> bool {
        self == Column::Quiet
    }
}

/// Counts over a list of current sessions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tally {
    pub n: u32,
    pub run: u32,
    pub warm: u32,
    pub held: u32,
    pub wait: u32,
    pub stall: u32,
    pub cold: u32,
    pub idle: u32,
    /// Held plus waiting, without the parked ones: what needs you now.
    pub need: u32,
    /// Waiting more than a day (model::PARKED): counted in `wait`, not in `need`.
    #[serde(default)]
    pub parked: u32,
    /// Tokens that going back to the cold, heavy chats would re-read.
    pub stake: u64,
    pub fresh_count: u32,
}

pub fn tally<'a>(list: impl IntoIterator<Item = &'a Session>, now: f64) -> Tally {
    let mut t = Tally::default();
    for s in list {
        t.n += 1;
        match s.lamp(now) {
            Lamp::Run => t.run += 1,
            Lamp::Warm => t.warm += 1,
            Lamp::Held => t.held += 1,
            Lamp::Wait => t.wait += 1,
            Lamp::Stall => t.stall += 1,
            Lamp::Cold => t.cold += 1,
            Lamp::Idle => t.idle += 1,
        }
        if s.needs_fresh(now) {
            t.stake += s.ctx;
            t.fresh_count += 1;
        }
        if s.parked(now) {
            t.parked += 1;
        }
    }
    t.need = (t.held + t.wait).saturating_sub(t.parked);
    t
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lane {
    pub machine: String,
    /// Session keys per column, in display order.
    pub cells: Vec<Vec<String>>,
    pub tally: Tally,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Board {
    pub lanes: Vec<Lane>,
    /// Sessions per column across every lane.
    pub column_counts: Vec<u32>,
    /// Context summed over the cold column.
    pub cold_ctx: u64,
}

/// Current sessions on one machine (not stale, not hidden), newest first.
pub fn current<'a>(snap: &'a Snapshot, machine: &str, hidden: &HashSet<String>) -> Vec<&'a Session> {
    snap.machine(machine)
        .map(|m| m.sessions.iter().filter(|s| !s.stale && !hidden.contains(&s.key)).collect())
        .unwrap_or_default()
}

/// The whole board. `only` limits it to one machine (a torn-off lane window).
pub fn build(snap: &Snapshot, now: f64, hidden: &HashSet<String>, only: Option<&str>) -> Board {
    let mut board = Board { column_counts: vec![0; 5], ..Default::default() };
    for m in snap.machines.iter().filter(|m| only.is_none_or(|o| o == m.name)) {
        let cur = current(snap, &m.name, hidden);
        let mut cells: Vec<Vec<&Session>> = vec![Vec::new(); 5];
        for s in &cur {
            cells[Column::of(s.lamp(now)).index()].push(s);
        }
        // Warm: the one about to go cold first.
        cells[Column::Warm.index()].sort_by_key(|s| s.cache_left(now));
        // Needs you: parked chats (waiting more than a day) after the ones that need you now. A stable sort, so
        // each group keeps its order.
        cells[Column::Need.index()].sort_by_key(|s| s.parked(now));
        for (i, c) in cells.iter().enumerate() {
            // Column heads count what needs you; a parked chat is there but not counted.
            board.column_counts[i] += c.iter().filter(|s| !(i == Column::Need.index() && s.parked(now))).count() as u32;
        }
        board.cold_ctx += cells[Column::Cold.index()].iter().map(|s| s.ctx).sum::<u64>();
        board.lanes.push(Lane {
            machine: m.name.clone(),
            cells: cells.iter().map(|c| c.iter().map(|s| s.key.clone()).collect()).collect(),
            tally: tally(cur.iter().copied(), now),
        });
    }
    board
}

impl Board {
    pub fn lane(&self, machine: &str) -> Option<&Lane> {
        self.lanes.iter().find(|l| l.machine == machine)
    }
    /// Where a key sits: (lane index, column, row).
    pub fn locate(&self, key: &str) -> Option<(usize, Column, usize)> {
        for (li, lane) in self.lanes.iter().enumerate() {
            for (ci, cell) in lane.cells.iter().enumerate() {
                if let Some(r) = cell.iter().position(|k| k == key) {
                    return Some((li, Column::ALL[ci], r));
                }
            }
        }
        None
    }
    /// Reading order for j/k: lane by lane, column by column, `cap(column)` rows per cell unless the lane is open,
    /// folded lanes skipped.
    pub fn order(&self, cap: impl Fn(Column) -> usize, open: &HashSet<String>, shut: &HashSet<String>) -> Vec<String> {
        let mut out = Vec::new();
        for lane in &self.lanes {
            if shut.contains(&lane.machine) {
                continue;
            }
            for (ci, cell) in lane.cells.iter().enumerate() {
                let n = if open.contains(&lane.machine) { cell.len() } else { cell.len().min(cap(Column::ALL[ci])) };
                out.extend(cell[..n].iter().cloned());
            }
        }
        out
    }
    /// h/l: the nearest non-empty column in `dir` within the same lane, keeping the row when it can.
    pub fn across(&self, key: Option<&str>, dir: i32) -> Option<String> {
        let (li, col, row) = match key.and_then(|k| self.locate(k)) {
            Some((li, c, r)) => (li, c.index() as i32, r),
            None => (0, -1, 0),
        };
        let lane = self.lanes.get(li)?;
        let mut i = col + dir;
        while (0..5).contains(&i) {
            let cell = &lane.cells[i as usize];
            if !cell.is_empty() {
                return Some(cell[row.min(cell.len() - 1)].clone());
            }
            i += dir;
        }
        None
    }
}
