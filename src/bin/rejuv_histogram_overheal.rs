extern crate wow_combat_log;
extern crate chrono;

use std::fs::File;
use std::io::BufReader;
use std::collections::HashMap;
use std::fmt;
use chrono::Duration;

static REJUV_AURAS: &'static [u32] = &[
    774, // Rejuv
    155777, // Rejuv (Germ)
    ];

#[derive(Debug, Clone, Default)]
struct RestoComputation<'a> {
    map: HashMap<&'a str, [usize; 2]>,
    histo: [(u64, u64, u64); 32],
    player: &'a str, 
}

impl<'a> RestoComputation<'a> {
    fn new(player: &'a str) -> Self {
        RestoComputation {player: player, .. Default::default() }
    }

    fn reset_stats(&mut self) {
        self.histo = Default::default();
    }

    fn parse_entry(&mut self, log: &wow_combat_log::Entry<'a>, filter_start_time: Duration) {
        use wow_combat_log::Entry::*;
        use wow_combat_log::AuraType::*;

        if log.base().is_none() {
            return;
        }
        let base = log.base().unwrap();
        if base.src.name != self.player {
            return;
        }
        let entry = self.map.entry(log.base().unwrap().dst.id).or_insert([0, 0]);
        match *log {
            Aura { ty, id, .. } if REJUV_AURAS.contains(&id) && ty != Remove => {
                let i = if id == REJUV_AURAS[0] { 0 } else { 1 };
                entry[i] = 0;
            },

            Heal { id, heal: total_heal, overheal, .. } if REJUV_AURAS.contains(&id) => {
                if log.timestamp() < filter_start_time {
                    return;
                }

                let i = if id == REJUV_AURAS[0] { 0 } else { 1 };
                self.histo[entry[i]].0 += overheal;
                self.histo[entry[i]].1 += total_heal;
                self.histo[entry[i]].2 += 1;
                if entry[i] < self.histo.len() {
                    entry[i] += 1;
                }
            },
            _ => ()
        }
    }
}

impl<'a> fmt::Display for RestoComputation<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let overheal = self.histo.iter().map(|x| x.0).sum::<u64>() as f64;
        let total = self.histo.iter().map(|x| x.1).sum::<u64>() as f64;
        writeln!(f, "total rejuv healing {} ({:.2}% overheal)", total, overheal/total * 100.)?;
        write!(f, "Overheal by tick: ")?;
        for i in &self.histo {
            if i.1 == 0 { break }
            write!(f, "{:.2} ({}), ", i.0 as f64/i.1 as f64 * 100., i.2)?;
        }
        Ok(())
    }
}

impl<'a, 'b, 'c> std::ops::SubAssign<&'b RestoComputation<'c>> for RestoComputation<'a> {
    fn sub_assign(&mut self, rhs: &'b RestoComputation<'c>) {
        for (i, j) in self.histo.iter_mut().zip(rhs.histo.iter()) {
            i.0 -= j.0;
            i.1 -= j.1;
            i.2 -= j.2;
        }
    }
}

fn main() {
    let read = BufReader::new(File::open(std::env::args().nth(1).unwrap()).unwrap());
    let player = std::env::args().nth(2).unwrap();
    let intern = wow_combat_log::Interner::default();
    let start = std::env::args().nth(3).map(|x| Duration::seconds(x.parse().unwrap())).unwrap_or(Duration::zero());
    let end = std::env::args().nth(4).map(|x| Duration::seconds(x.parse().unwrap())).unwrap_or(Duration::max_value());

    let iter = wow_combat_log::iter(&intern, read);
    let iter = iter.take_while(|x| x.timestamp() < end);
    let mut encounter_start = None;
    let mut total = RestoComputation::new(&player);
    let mut encounter = total.clone();
    let mut kills = total.clone();
    let mut bosses = total.clone();

    for log in iter {
        use wow_combat_log::Entry::*;
        match log {
            EncounterStart {..} => {
                encounter_start = Some(log.timestamp());
                bosses -= &encounter;
                kills -= &encounter;
                encounter.reset_stats();
            },
            EncounterEnd {name, kill, ..} => {
                if let Some(s) = encounter_start {
                    println!("duration: {}, start: {}, {}, kill: {}", (log.timestamp() - s).num_seconds(), s.num_seconds(), name, kill);
                    println!("{}", encounter);
                    println!("");
                    encounter_start = None;
                }
                if !kill {
                    kills -= &encounter;
                }
                encounter.reset_stats();
            },
            _ => ()
        }
        encounter.parse_entry(&log, start);
        total.parse_entry(&log, start);
        kills.parse_entry(&log, start);
        bosses.parse_entry(&log, start);
    }
    bosses -= &encounter;
    kills -= &encounter;

    println!("-------");
    println!("");
    println!("Log total:");
    println!("{}", total);
    println!("");
    println!("Boss total:");
    println!("{}", bosses);
    println!("");
    println!("Kill total:");
    println!("{}", kills);
}
