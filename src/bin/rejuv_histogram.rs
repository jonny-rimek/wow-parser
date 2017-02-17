extern crate wow_combat_log;
extern crate chrono;

use std::fs::File;
use std::io::BufReader;
use std::io::{Seek, SeekFrom};
use std::collections::HashMap;
use std::fmt;
use chrono::Duration;

static REJUV_AURAS: &'static [u32] = &[
    774, // Rejuv
    155777, // Rejuv (Germ)
    ];

#[derive(Debug, Clone, Default)]
struct RestoComputation<'a> {
    map: HashMap<&'a str, [Duration; 2]>,
    histo: [u64; 32],
    player: &'a str, 
}

impl<'a> RestoComputation<'a> {
    fn new(player: &'a str) -> Self {
        RestoComputation {player: player, .. Default::default() }
    }

    fn reset_stats(&mut self) {
        self.histo = Default::default();
    }

    fn parse_entry(&mut self, log: wow_combat_log::Entry<'a>, filter_start_time: Duration) {
        use wow_combat_log::Entry::*;
        use wow_combat_log::AuraType::*;

        if log.base().is_none() {
            return;
        }
        let base = log.base().unwrap();
        if base.src.name != self.player {
            return;
        }
        let entry = self.map.entry(log.base().unwrap().dst.id).or_insert([log.timestamp(), log.timestamp()]);
        match log {
            Aura { ty, id, .. } if REJUV_AURAS.contains(&id) && ty != Remove => {
                let i = if id == REJUV_AURAS[0] { 0 } else { 1 };
                entry[i] = log.timestamp();
            },

            Heal { id, heal: total_heal, overheal, ty, .. } if REJUV_AURAS.contains(&id) => {
                if log.timestamp() < filter_start_time {
                    return;
                }

                let heal = total_heal - overheal;
                let i = if id == REJUV_AURAS[0] { 0 } else { 1 };
                let secs = (log.timestamp() - entry[i]).num_seconds();
                
                if secs >= 0 && secs < self.histo.len() as i64 {
                    self.histo[secs as usize] += heal;
                } else if secs >= 0 {
                    *self.histo.last_mut().unwrap() += heal;
                }
            },
            _ => ()
        }
    }
}

impl<'a> fmt::Display for RestoComputation<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let total = self.histo.iter().cloned().sum::<u64>() as f64;
        writeln!(f, "total rejuv healing {}", total)?;
        write!(f, "Percentage by seconds: ")?;
        for &i in &self.histo {
            write!(f, "{:.2}, ", i as f64 * 100. / total)?;
        }
        Ok(())
    }
}

impl<'a, 'b, 'c> std::ops::SubAssign<&'b RestoComputation<'c>> for RestoComputation<'a> {
    fn sub_assign(&mut self, rhs: &'b RestoComputation<'c>) {
        for (i, &j) in self.histo.iter_mut().zip(rhs.histo.iter()) {
            *i -= j;
        }
    }
}

fn main() {
    let mut read = BufReader::new(File::open(std::env::args().nth(1).unwrap()).unwrap());
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
        encounter.parse_entry(log, start);
        total.parse_entry(log, start);
        kills.parse_entry(log, start);
        bosses.parse_entry(log, start);
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
