extern crate chrono;
extern crate itertools;
#[macro_use]
extern crate lazy_static;

#[cfg(feature = "wcl")]
extern crate json;
#[cfg(feature = "wcl")]
extern crate reqwest;

mod intern;
mod collect_tuple;
#[cfg(feature = "wcl")]
pub mod wcl;

use chrono::Duration;
use chrono::NaiveDateTime;
use chrono::NaiveDate;
use chrono::NaiveTime;
use chrono::Datelike;
pub use intern::Interner;
use std::io::BufRead;
use collect_tuple::OrPanic;
use itertools::Itertools;


#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Object<'a> {
    pub name: &'a str,
    pub id: &'a str,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct BaseInfo<'a> {
    pub timestamp: Duration,
    pub src: Object<'a>,
    pub src_flags1: u32,
    pub src_flags2: u32,
    pub dst: Object<'a>,
    pub dst_flags1: u32,
    pub dst_flags2: u32,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum AuraType { Apply, Refresh, Remove, Stack }
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum HealType { Heal, Periodic }

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Entry<'a> {
    Aura { ty: AuraType, base: BaseInfo<'a>, id: u32, aura: &'a str, flags: u8, buff: bool },
    Heal { ty: HealType, base: BaseInfo<'a>, id: u32, spell: &'a str, flags: u8, hp: u64, maxhp: u64, heal: u64, overheal: u64, crit: bool },


    // followed by: ???, (talents), (pvp talents), [artifact info], [gear], [buffs]
    Info { ts: Duration, id: &'a str, strength: u32, agi: u32, sta: u32, int: u32, dodge: u32, parry: u32, block: u32, critm: u32, critr: u32, crits: u32, spd: u32, steal: u32, hastem: u32, hastr: u32, hastes: u32, avd: u32, mastery: u32, versm: u32, versr: u32, verss: u32, armor: u32,

           // (source id, aura id)
           auras: Vec<(&'a str, u32)>,
    },
    ChallengeStart { ts: Duration, id: u32 },
    ChallengeEnd { ts: Duration, id: u32 },
    EncounterStart { ts: Duration, name: &'a str, id: u32, difficulty: u16, }, // , difficulty/dungeon?, num players
    EncounterEnd { ts: Duration, name: &'a str, id: u32, difficulty: u16, kill: bool },
    Unknown(Duration, &'a str),
}

lazy_static!{ static ref YEAR: i32 = chrono::UTC::today().year(); }

fn parse_ts(line: &str) -> (NaiveDateTime, &str) {
    let OrPanic((ts_str, line)) = line.splitn(2, "  ").collect();
    let OrPanic((date, t)) = ts_str.splitn(2, " ").collect();
    let OrPanic((m, d)) = date.splitn(2, "/").collect();
    let date = NaiveDate::from_ymd(*YEAR, m.parse().unwrap(), d.parse().unwrap());
    (date.and_time(NaiveTime::parse_from_str(t, "%H:%M:%S%.f").unwrap()), line)
}

fn parse_hex(x: &str) -> u32 {
    u32::from_str_radix(&x[2..], 16).unwrap()
}

fn parse_quote(x: &str) -> (&str, &str) {
    if x.starts_with('"') {
        let OrPanic((_, a, b)) = x.splitn(3, '"').collect();
        assert!(b.starts_with(','));
        let b = &b[1..];
        (a, b)
    } else {
        let OrPanic((a, b)) = x.splitn(2, ',').collect();
        (a, b)
    }
}

fn parse_base<'a, 'b>(intern: &'a Interner, line: &'b str, timestamp: Duration) -> (BaseInfo<'a>, &'b str) {
    let OrPanic((srcid, line)) = line.splitn(2, ',').collect();
    let (srcname, line) = parse_quote(line);
    let OrPanic((srcf1, srcf2, dstid, line)) = line.splitn(4, ',').collect();
    let (dstname, line) = parse_quote(line);
    let OrPanic((dstf1, dstf2, line)) = line.splitn(3, ',').collect();
    (BaseInfo {
        timestamp: timestamp,
        src: Object { name: intern.intern(srcname), id: intern.intern(srcid) },
        src_flags1: parse_hex(srcf1), src_flags2: parse_hex(srcf2),
        dst: Object { name: intern.intern(dstname), id: intern.intern(dstid) },
        dst_flags1: parse_hex(dstf1), dst_flags2: parse_hex(dstf2),
    }, line)
}

pub fn parse_line<'a>(intern: &'a Interner, line: &str, start_time: NaiveDateTime) -> Entry<'a> {
    let (ts, line) = parse_ts(line);
    let OrPanic((ty, line)) = line.splitn(2, ",").collect();
    let dur = ts - start_time;
    match ty {
        "SPELL_AURA_APPLIED" | "SPELL_AURA_REMOVED" | "SPELL_AURA_REFRESH" |
        "SPELL_AURA_APPLIED_DOSE" | "SPELL_AURA_REMOVED_DOSE" => {
            let (base, line) = parse_base(intern, line, dur);
            let OrPanic((id, line)) = line.splitn(2, ',').collect();
            let (name, line) = parse_quote(line);
            let OrPanic((flag, buff)) = line.splitn(2, ",").collect();
            let name = intern.intern(name);
            let ty = match ty {
                "SPELL_AURA_APPLIED" => AuraType::Apply,
                "SPELL_AURA_APPLIED_DOSE" | "SPELL_AURA_REMOVED_DOSE" => AuraType::Stack,
                "SPELL_AURA_REMOVED" => AuraType::Remove,
                "SPELL_AURA_REFRESH" => AuraType::Refresh,
                _ => unreachable!(),
            };
            let buff = buff.trim() == "BUFF";
            Entry::Aura { ty: ty, base: base, id: id.parse().unwrap(), aura: name, flags: parse_hex(flag) as u8, buff: buff }
        },
        "SPELL_HEAL" | "SPELL_PERIODIC_HEAL" => {
            let (base, line) = parse_base(intern, line, dur);
            let OrPanic((id, line)) = line.splitn(2, ',').collect();
            let (name, line) = parse_quote(line);
            let OrPanic((flag, _someguid, _zeros, hp, maxhp,
                         _ap, _sp, _energytype, _energy, _energymax, _map_index_maybe,
                         _x, _y, _ilvl, heal, overheal, _zero, crit)) = line.splitn(18, ",").collect();
            let name = intern.intern(name);
            let ty = match ty {
                "SPELL_HEAL" => HealType::Heal,
                "SPELL_PERIODIC_HEAL" => HealType::Periodic,
                _ => unreachable!(),
            };
            Entry::Heal { ty: ty, base: base, id: id.parse().unwrap(), spell: name, flags: parse_hex(flag) as u8,
                          hp: hp.parse().unwrap(), maxhp: maxhp.parse().unwrap(), heal: heal.parse().unwrap(), overheal: overheal.parse().unwrap(),
                          crit: crit.trim() == "1"  }
        },
        "COMBATANT_INFO" => {
            let OrPanic((id, strength, agi, sta, int, dodge, parry, block,
                         critm, critr, crits, spd, steal,
                         hastem, hastr, hastes, avd, mastery,
                         versm, versr, verss, armor, line)) = line.splitn(23, ',').collect();
            let OrPanic((_talents, _artifact, _gear, auras)) = line.split('[').collect();
            let auras = auras.trim().trim_right_matches(']');
            let auras = auras.split(',').tuples()
                .map(|(src, aura)| (intern.intern(src), aura.parse().unwrap())).collect();
            Entry::Info { ts: dur, id: intern.intern(id), strength: strength.parse().unwrap(), agi: agi.parse().unwrap(), sta: sta.parse().unwrap(), int: int.parse().unwrap(),
                          dodge: dodge.parse().unwrap(), parry: parry.parse().unwrap(), block: block.parse().unwrap(),
                          critm: critm.parse().unwrap(), critr: critr.parse().unwrap(), crits: crits.parse().unwrap(), spd: spd.parse().unwrap(), steal: steal.parse().unwrap(),
                          hastem: hastem.parse().unwrap(), hastr: hastr.parse().unwrap(), hastes: hastes.parse().unwrap(), avd: avd.parse().unwrap(), mastery: mastery.parse().unwrap(),
                          versm: versm.parse().unwrap(), versr: versr.parse().unwrap(), verss: verss.parse().unwrap(), armor: armor.parse().unwrap(),
                          auras: auras,
            }
        },
        "CHALLENGE_MODE_START" => {
            let OrPanic((id, _line)) = line.splitn(2, ',').collect();
            Entry::ChallengeStart { ts: dur, id: id.parse().unwrap() }
        },
        "CHALLENGE_MODE_END" => {
            let OrPanic((id, _line)) = line.splitn(2, ',').collect();
            Entry::ChallengeEnd { ts: dur, id: id.parse().unwrap() }
        },
        "ENCOUNTER_START" => {
            let OrPanic((id, line)) = line.splitn(2, ',').collect();
            let (name, line) = parse_quote(line);
            let OrPanic((difficulty, _players)) = line.splitn(2, ',').collect();
            Entry::EncounterStart { ts: dur, name: intern.intern(name), id: id.parse().unwrap(), difficulty: difficulty.parse().unwrap() }
        },
        "ENCOUNTER_END" => {
            let OrPanic((id, line)) = line.splitn(2, ',').collect();
            let (name, line) = parse_quote(line);
            let OrPanic((difficulty, _players, kill)) = line.splitn(3, ',').collect();
            Entry::EncounterEnd { ts: dur, name: intern.intern(name), id: id.parse().unwrap(), difficulty: difficulty.parse().unwrap(), kill: kill.trim() == "1" }
        },
        x => Entry::Unknown(dur, intern.intern(x)),
    }
}

#[derive(Debug, Clone)]
pub struct Iter<'a, R: BufRead> {
    intern: &'a Interner,
    read: R,
    start: NaiveDateTime,
    nextline: String,
}

pub fn iter<R: BufRead>(intern: &Interner, mut read: R) -> Iter<R> {
    let mut s = String::new();
    read.read_line(&mut s).unwrap();
    let start = parse_ts(&s).0;
    Iter { intern: intern, read: read, start: start, nextline: s }
}

impl<'a, R: BufRead> Iterator for Iter<'a, R> {
    type Item = Entry<'a>;
    fn next(&mut self) -> Option<Entry<'a>> {
        if self.nextline.len() == 0 { return None }
        let ret = parse_line(self.intern, &self.nextline, self.start);
        self.nextline.clear();
        self.read.read_line(&mut self.nextline).unwrap();
        Some(ret)
    }
}

impl<'a> Entry<'a> {
    pub fn base(&self) -> Option<&BaseInfo<'a>> {
        match *self {
            Entry::Aura { ref base, .. } => Some(base),
            Entry::Heal { ref base, .. } => Some(base),
            _ => None
        }
    }
    pub fn timestamp(&self) -> Duration {
        use Entry::*;
        match *self {
            Aura { ref base, .. } => base.timestamp,
            Heal { ref base, .. } => base.timestamp,
            Info { ts, .. } => ts,
            ChallengeStart { ts, .. } => ts,
            ChallengeEnd { ts, .. } => ts,
            EncounterStart { ts, ..} => ts,
            EncounterEnd { ts, ..} => ts,
            Unknown(t, _) => t,
        }
    }
}
