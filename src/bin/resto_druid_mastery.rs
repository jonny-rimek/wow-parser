extern crate wow_combat_log;
extern crate chrono;
extern crate clap;

use std::fs::File;
use std::io::BufReader;
use std::collections::{HashMap, HashSet};
use std::fmt;
use chrono::Duration;
use clap::{Arg, App};
use wow_combat_log::Entry;

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

static MASTERY_NAMES: &'static [(u32, &'static str)] = &[
    (33763, "LB"),
    (774, "Rejuv"),
    (155777, "Germ"),
    (8936, "Regrowth"),
    (48438, "WG"),
    (207386, "SB"),
    (200389, "Cult"),
    (102352, "CW"),
    (22842, "Frenzied"),
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
const SPELL_REGROWTH: u32 = 8936;
const SPELL_TRANQ: u32 = 157982;
const MASTERY_2PC: u32 = 4000;

pub fn find_init_mastery<'a, I: Iterator<Item=Entry<'a>>>(iter: I, player: &str) -> Option<(&'a str, u32)> {
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


#[derive(Default, Debug, Clone)]
pub struct RestoComputation<'a> {
    map: HashMap<&'a str, (HashSet<u32>, Duration)>,
    total_healing: u64,
    total_unmastery_healing: u64,
    total_uncrit_healing: u64,
    mastery_healing: u64,
    living_seed_healing: u64,
    regrowth_healing: u64,
    tranq_healing: u64,
    rejuv_healing: u64,
    healing_2pc: u64,
    healing_2pc_added: u64,
    under_2pc: bool,
    player_id: &'a str,
    cur_mastery: u32,
    total_healing_per: [u64; 14],
    total_healing_per_unmast: [u64; 14],
    hot_mastery_healing_added: HashMap<u32, u64>,
}

impl<'a> RestoComputation<'a> {
    pub fn new(player_id: &'a str, starting_mastery: u32) -> Self {
        RestoComputation {
            player_id: player_id, cur_mastery: starting_mastery,
            ..Default::default()
        }
    }

    pub fn reset_stats(&mut self) {
        let prev = std::mem::replace(self, Default::default());
        *self = RestoComputation {
            player_id: prev.player_id,
            cur_mastery: prev.cur_mastery,
            map: prev.map,
            ..Default::default()
        }
    }

    pub fn parse_entry(&mut self, log: &wow_combat_log::Entry<'a>, filter_start_time: Duration) {
        use wow_combat_log::Entry::*;
        use wow_combat_log::AuraType::*;

        if let Info { id, mastery, ref auras, .. } = *log {
            let entry = self.map.entry(id).or_insert((HashSet::new(), log.timestamp()));
            let player_id = self.player_id;
            if player_id == id {
                self.cur_mastery = mastery;
                if auras.contains(&(self.player_id, AURA_2PC)) {
                    self.cur_mastery -= MASTERY_2PC;
                    self.under_2pc = true;
                }
            }
            entry.0 = auras.iter()
                .filter(|&&(src, aura)| src == player_id && MASTERY_AURAS.contains(&aura))
                .map(|&(_, aura)| aura).collect();
            entry.1 = log.timestamp();
        }

        if log.base().is_none() {
            return;
        }
        let base = log.base().unwrap();
        if base.src.id != self.player_id {
            return;
        }
        let entry = self.map.entry(log.base().unwrap().dst.id).or_insert((HashSet::new(), log.timestamp()));
        let diff = log.timestamp() - entry.1;

        // If we haven't seen anything from them for 10 seconds,
        // assume they left the zone and may have lost all their buffs
        if diff > Duration::seconds(10) {
            entry.0.clear();
        } else {
            entry.1 = log.timestamp();
        }
        match *log {
            Aura { ty, id, .. } if MASTERY_AURAS.contains(&id) => {
                match ty {
                    Apply | Refresh => {
                        entry.0.insert(id);
                    },
                    Remove => {
                        entry.0.remove(&id);
                    },
                    _ => (),
                }
                entry.1 = log.timestamp();
            },
            Aura { ty, id: AURA_2PC, .. } => {
                self.under_2pc = ty != Remove;
            },
            Heal { id, heal: total_heal, overheal, crit, ty, .. } => {
                if log.timestamp() < filter_start_time {
                    return;
                }

                let heal = total_heal - overheal;
                let stacks = entry.0.len();
                let mastery = self.cur_mastery + if self.under_2pc { MASTERY_2PC } else { 0 };
                let mastery = (mastery as f64 /666.6+4.8)/100.;
                let unmast = ((heal as f64) / (1. + stacks as f64 * mastery)) as u64;
                let uncrit_heal = if crit { total_heal / 2 } else { total_heal }; // TODO /2 ignores drape and tauren
                let uncrit_heal = std::cmp::min(uncrit_heal, heal);
                self.total_healing += heal;
                if REJUV_AURAS.contains(&id) {
                    self.rejuv_healing += heal;
                }
                if MASTERY_AURAS.contains(&id) || OTHER_HEALS.contains(&id) {
                    self.total_healing_per[stacks] += heal;
                    self.total_healing_per_unmast[stacks] += unmast;
                    self.mastery_healing += (stacks as u64) * unmast;
                    self.total_unmastery_healing += unmast;

                    for &aura in &entry.0 {
                        // Only measure the contribution to other heals
                        if aura != id {
                            let added = (unmast as f64 * mastery) as u64;
                            *self.hot_mastery_healing_added.entry(aura).or_insert(0) += added;
                        }
                    }

                    if self.under_2pc {
                        let added = (stacks as f64 * unmast as f64 * MASTERY_2PC as f64 /666.6 / 100.) as u64;
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
        write!(f, "scale_mastery_frac: {:.6}; scale_living_seed: {:.6}; scale_regrowth: {:.6}; scale_tranq: {:.6}; scale_rejuv: {:.6};\n  scale_2pc: {:.6}; scale_2pc_added: {:.6};\n",
                    self.mastery_healing as f64 / self.total_unmastery_healing as f64,
                    self.living_seed_healing as f64 / self.total_uncrit_healing as f64,
                    self.regrowth_healing as f64 / self.total_uncrit_healing as f64,
                    self.tranq_healing as f64 / self.total_healing as f64,
                    self.rejuv_healing as f64 / self.total_healing as f64,
                    self.healing_2pc as f64/self.total_healing as f64,
                    self.healing_2pc_added as f64/self.total_healing as f64,
                    )?;
        writeln!(f, "Mastery stack healing on other heals: ")?;
        for &(aura, name) in MASTERY_NAMES {
            let added = self.hot_mastery_healing_added.get(&aura).map(|x| *x).unwrap_or(0);
            write!(f, "{}: {:.6},  ", name, added as f64 / self.total_healing as f64)?;
        }
        Ok(())
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
        self.healing_2pc -= rhs.healing_2pc;
        self.healing_2pc_added -= rhs.healing_2pc_added;
        for (i, &j) in self.total_healing_per.iter_mut().zip(rhs.total_healing_per.iter()) {
            *i -= j;
        }
        for (i, &j) in self.total_healing_per_unmast.iter_mut().zip(rhs.total_healing_per_unmast.iter()) {
            *i -= j;
        }
        for (aura, &heal) in rhs.hot_mastery_healing_added.iter() {
            *self.hot_mastery_healing_added.get_mut(aura).unwrap() -= heal;
        }
    }
}

fn run<'a, I: Iterator<Item=Entry<'a>>, F: Fn(Option<&str>) -> I>(player: &str, start: Duration, end: Duration, get_iter: F) {
    
    let (pid, cur_mastery) = find_init_mastery(get_iter(None), player).unwrap();
    let iter = get_iter(Some(player));
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

#[cfg(feature = "wcl")]
pub fn wcl_iter<'a>(intern: &'a wow_combat_log::Interner, log: &str, api_key: &str,
                    skip: bool, name: Option<&str>) -> wow_combat_log::wcl::Iter<'a> {
    wow_combat_log::wcl::iter(intern, log, api_key, skip, name)
}

#[cfg(not(feature = "wcl"))]
pub fn wcl_iter<'a>(_: &'a wow_combat_log::Interner, _: &str, _: &str,
                    _: bool, _: Option<&str>) -> wow_combat_log::Iter<'a, BufReader<File>> {
    unreachable!()
}

fn main() {
    let app = App::new("resto druid mastery");
    let app = if cfg!(feature = "wcl") {
        app.arg(Arg::with_name("API key").long("wcl").takes_value(true).help("warcraftlogs API key"))
    } else {
        app
    };
    let matches = app
        .arg(Arg::with_name("File/WCL ID").required(true).help("Log file or WCL log ID"))
        .arg(Arg::with_name("Player").required(true).help("Player name (as reported in log)"))
        .arg(Arg::with_name("Start").help("Start time in seconds from start of log"))
        .arg(Arg::with_name("End").help("End time in seconds from start of log"))
        .get_matches();
    let player = matches.value_of("Player").unwrap();
    let intern = wow_combat_log::Interner::default();
    let start = matches.value_of("Start").map(|x| Duration::seconds(x.parse().unwrap())).unwrap_or(Duration::zero());
    let end = matches.value_of("End").map(|x| Duration::seconds(x.parse().unwrap())).unwrap_or(Duration::max_value());
    let input = matches.value_of("File/WCL ID").unwrap();
    if let Some(api) = matches.value_of("API key") {
        run(player, start, end, |player|
            wcl_iter(&intern, input, api, player.is_none(), player)
        );
    } else {
        run(player, start, end, |_|
            wow_combat_log::iter(&intern, BufReader::new(File::open(input).unwrap()))
        );
    }
}
