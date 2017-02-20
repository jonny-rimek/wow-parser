extern crate wow_combat_log;
extern crate chrono;

use std::fs::File;
use std::io::BufReader;
use std::io::{Seek, SeekFrom};
use std::collections::HashMap;
use std::fmt;
use chrono::Duration;

static MASTERY_AURAS: &'static [u32] = &[
    33763, // Lifebloom
    774, // Rejuv
    155777, // Rejuv (Germ)
    8936, // Regrowth,
    48438, // WG
    207386, // Spring Blossoms
    200389, // Cultivation
    102352, // Cenarion Ward
    22842, // Frenzied Regen (no really, it counts)
    ];

// not renewal(108238), not ysera's gift(145109/10), not trinkets
//
// Living seed is not itself affected by mastery, but the heal its
// strength is based on _is_. The ideal computation would be to use
// the mastery stacks (and rating) from when the heal was created, but
// to use the overheal/etc values that we only know when it goes off.
static OTHER_HEALS: &'static [u32] = &[
    157982, // Tranq
    18562, // Swiftmend
    33778, // Lifebloom bloom
    5185, // HT
    81269, // Efflo
    189800, // Nature's Essence (WG insta heal)
    189853, // Dreamwalker
    48503, // Living seed
    ];

static REJUV_AURAS: &'static [u32] = &[
    774, // Rejuv
    155777, // Rejuv (Germ)
    ];

static LIVING_SEED_HEALS: &'static [u32] = &[5185, 8936, 18562];
const AURA_2PC: u32 = 232378;
const AURA_CULT: u32 = 200389;
const SPELL_REGROWTH: u32 = 8936;
const SPELL_TRANQ: u32 = 157982;
const MASTERY_2PC: u32 = 4000;

fn find_init_mastery<'a, R: std::io::BufRead>(iter: wow_combat_log::Iter<'a, R>, player: &str) -> Option<(&'a str, u32)> {
    let mut map = HashMap::new();
    let mut player_id = None;
    for log in iter {
        if player_id.is_none() {
            if let Some(base) = log.base() {
                let id;
                if base.src.name == player {
                    id = base.src.id;
                } else if base.dst.name == player {
                    id = base.dst.id;
                } else {
                    continue;
                }
                if let Some(v) = map.get(id) {
                    return Some((id, *v));
                }
                player_id = Some(id);
            }
        }
        match log {
            wow_combat_log::Entry::Info { id, mastery, .. } => {
                if let Some(pid) = player_id {
                    if pid == id {
                        return Some((pid, mastery));
                    } else {
                        continue
                    }
                }
                map.entry(id).or_insert(mastery);
            },
            _ => (),
        }
    }
    //None
    Some((player_id.unwrap(), 8773))
}


#[derive(Debug, Clone)]
struct RestoComputation<'a> {
    map: HashMap<&'a str, (u32, Duration, bool)>,
    total_healing: u64,
    total_unmastery_healing: u64,
    total_uncrit_healing: u64,
    mastery_healing: u64,
    living_seed_healing: u64,
    regrowth_healing: u64,
    tranq_healing: u64,
    rejuv_healing: u64,
    cult_healing: u64,
    healing_2pc: u64,
    healing_2pc_added: u64,
    under_2pc: bool,
    player_id: &'a str,
    cur_mastery: u32,
}

impl<'a> RestoComputation<'a> {
    fn new(player_id: &'a str, starting_mastery: u32) -> Self {
        RestoComputation {
            map: HashMap::new(),
            total_healing: 0,
            total_unmastery_healing: 0,
            total_uncrit_healing: 0,
            mastery_healing: 0,
            living_seed_healing: 0,
            regrowth_healing: 0,
            tranq_healing: 0,
            rejuv_healing: 0,
            cult_healing: 0,
            healing_2pc: 0,
            healing_2pc_added: 0,
            under_2pc: false,
            player_id: player_id, cur_mastery: starting_mastery }
    }

    fn reset_stats(&mut self) {
        self.total_healing = 0;
        self.total_unmastery_healing = 0;
        self.total_uncrit_healing = 0;
        self.mastery_healing = 0;
        self.living_seed_healing = 0;
        self.regrowth_healing = 0;
        self.tranq_healing = 0;
        self.rejuv_healing = 0;
        self.cult_healing = 0;
        self.healing_2pc = 0;
        self.healing_2pc_added = 0;
    }

    fn parse_entry(&mut self, log: wow_combat_log::Entry<'a>, filter_start_time: Duration) {
        use wow_combat_log::Entry::*;
        use wow_combat_log::AuraType::*;

        if let Info { id, mastery, .. } = log {
            if self.player_id == id {
                self.cur_mastery = mastery;
            }
        }

        if log.base().is_none() {
            return;
        }
        let base = log.base().unwrap();
        if base.src.id != self.player_id {
            return;
        }
        let entry = self.map.entry(log.base().unwrap().dst.id).or_insert((0, log.timestamp(), false));
        let diff = log.timestamp() - entry.1;
        if diff > Duration::seconds(10) { // buff drops can get lost (if they happen offzone say), no buff lasts 40s (we still don't track this _properly_, but good enough)
            entry.0 = 0;
        }
        match log {
            Aura { ty, id, .. } if MASTERY_AURAS.contains(&id) => {
                if entry.0 == 0 {
                    entry.1 = log.timestamp();
                }
                match ty {
                    Apply => entry.0 += 1,
                    Remove => entry.0 = entry.0.saturating_sub(1), 
                    _ => (),
                }
                entry.1 = log.timestamp();
                if id == AURA_CULT {
                    entry.2 = ty != Remove;
                }
            },
            Aura { ty, id: AURA_2PC, .. } => {
                self.under_2pc = ty != Remove;
            },
            Heal { id, heal: total_heal, overheal, crit, ty, .. } => {
                if log.timestamp() < filter_start_time {
                    return;
                }

                let heal = total_heal - overheal;
                let mastery = self.cur_mastery + if self.under_2pc { MASTERY_2PC } else { 0 }; 
                let unmast = ((heal as f64) / (1. + entry.0 as f64 * (mastery as f64 /666.6+4.8)/100.)) as u64;
                let uncrit_heal = if crit { total_heal / 2 } else { total_heal }; // TODO /2 ignores drape and tauren
                let uncrit_heal = std::cmp::min(uncrit_heal, heal);
                self.total_healing += heal;
                if REJUV_AURAS.contains(&id) {
                    self.rejuv_healing += heal;
                }
                if MASTERY_AURAS.contains(&id) || OTHER_HEALS.contains(&id) {
                    self.mastery_healing += (entry.0 as u64) * unmast;
                    self.total_unmastery_healing += unmast;
                    if entry.2 {
                        self.cult_healing += heal;
                    }

                    if self.under_2pc {
                        let added = (entry.0 as f64 * unmast as f64 * MASTERY_2PC as f64 /666.6 / 100.) as u64;
                        self.healing_2pc += heal;
                        self.healing_2pc_added += added;
                    }
                }

                self.total_uncrit_healing += uncrit_heal;
                if ty == wow_combat_log::HealType::Heal {
                    if LIVING_SEED_HEALS.contains(&id) {
                        self.living_seed_healing += uncrit_heal;
                    }
                    if id == SPELL_REGROWTH {
                        self.regrowth_healing += uncrit_heal;
                    }
                }
                if id == SPELL_TRANQ {
                    self.tranq_healing += heal;
                }
            },
            _ => ()
        }
    }
}

impl<'a> fmt::Display for RestoComputation<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "scale_mastery_frac: {:.6}; scale_living_seed: {:.6}; scale_regrowth: {:.6}; scale_tranq: {:.6}; scale_rejuv: {:.6};\n  scale_2pc: {:.6}; scale_2pc_added: {:.6}; scale_cult: {:.6}",
               self.mastery_healing as f64 / self.total_unmastery_healing as f64,
               self.living_seed_healing as f64 / self.total_uncrit_healing as f64,
               self.regrowth_healing as f64 / self.total_uncrit_healing as f64,
               self.tranq_healing as f64 / self.total_healing as f64,
               self.rejuv_healing as f64 / self.total_healing as f64,
               self.healing_2pc as f64/self.total_healing as f64,
               self.healing_2pc_added as f64/self.total_healing as f64,
               self.cult_healing as f64/self.total_healing as f64,
               )
    }
}

impl<'a, 'b, 'c> std::ops::SubAssign<&'b RestoComputation<'c>> for RestoComputation<'a> {
    fn sub_assign(&mut self, rhs: &'b RestoComputation<'c>) {
        self.total_healing -= rhs.total_healing;
        self.total_unmastery_healing -= rhs.total_unmastery_healing;
        self.total_uncrit_healing -= rhs.total_uncrit_healing;
        self.mastery_healing -= rhs.mastery_healing;
        self.living_seed_healing -= rhs.living_seed_healing;
        self.regrowth_healing -= rhs.regrowth_healing;
        self.tranq_healing -= rhs.tranq_healing;
        self.rejuv_healing -= rhs.rejuv_healing;
        self.cult_healing -= rhs.cult_healing;
        self.healing_2pc -= rhs.healing_2pc;
        self.healing_2pc_added -= rhs.healing_2pc_added;
    }
}

fn main() {
    let mut read = BufReader::new(File::open(std::env::args().nth(1).unwrap()).unwrap());
    let player = std::env::args().nth(2).unwrap();
    let intern = wow_combat_log::Interner::default();
    let start = std::env::args().nth(3).map(|x| Duration::seconds(x.parse().unwrap())).unwrap_or(Duration::zero());
    let end = std::env::args().nth(4).map(|x| Duration::seconds(x.parse().unwrap())).unwrap_or(Duration::max_value());
    let (pid, cur_mastery) = find_init_mastery(wow_combat_log::iter(&intern, &mut read), &player).unwrap();
    read.seek(SeekFrom::Start(0)).unwrap();

    let iter = wow_combat_log::iter(&intern, read);
    let iter = iter.take_while(|x| x.timestamp() < end);
    let mut encounter_start = None;
    let mut total = RestoComputation::new(pid, cur_mastery);
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
            EncounterEnd {name, kill, difficulty, ..} => {
                if let Some(s) = encounter_start {
                    println!("duration: {}, start: {}, {} ({}), kill: {}", (log.timestamp() - s).num_seconds(), s.num_seconds(), name, difficulty, kill);
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
