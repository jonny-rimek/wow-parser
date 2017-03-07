use intern::Interner;
use json::{JsonValue, self};
use reqwest::{Client, self};
use reqwest::Url;
use std::io::{BufReader, Read};
use chrono::Duration;
use std::collections::HashMap;
use std::str;

use {Entry, Object, BaseInfo, AuraType, HealType};

#[derive(Debug)]
pub struct Iter<'a> {
    intern: &'a Interner,
    chunk: Result<JsonValue, u64>,
    next: usize,
    client: Client,
    base_url: String,
    names: HashMap<isize, &'a str>,
    print: bool,
}

fn get_json(client: &Client, url: Url) -> JsonValue {
    let resp = client.get(url).send().unwrap();
    assert_eq!(resp.status(), &reqwest::StatusCode::Ok);
    let mut buf = vec![];
    BufReader::new(resp).read_to_end(&mut buf).unwrap();
    json::parse(str::from_utf8(&buf).unwrap()).unwrap()
}

fn parse_fights<'a>(intern: &'a Interner, log: &str, api_key: &str) -> (Client, HashMap<isize, &'a str>, u64, u64) {
    let client = Client::new().unwrap();
    let mut base_url = "https://www.warcraftlogs.com:443/v1/report/fights/".to_owned();
    base_url.push_str(log);
    let url = Url::parse_with_params(&base_url, &[("api_key", api_key)]).unwrap();
    let json = get_json(&client, url);
    let end = json["end"].as_u64().unwrap() - json["start"].as_u64().unwrap();
    let mut map = HashMap::new();
    for f in json["friendlies"].members() {
        map.insert(f["id"].as_isize().unwrap(),
                   intern.intern(f["name"].as_str().unwrap()));
    }
    let start = json["fights"].members().filter(|x| x["boss"] != 0).map(|x| x["start_time"].as_u64()).nth(0);
    let start = start.unwrap().unwrap();
    (client, map, end, start)
}

pub fn iter<'a>(intern: &'a Interner, log: &str, api_key: &str,
                skip_to_first_boss: bool, actorname: Option<&str>) -> Iter<'a> {
    let (client, names, end, first_boss) = parse_fights(intern, log, api_key);
    let mut base_url = "https://www.warcraftlogs.com:443/v1/report/events/".to_owned();
    base_url.push_str(log);
    let start = if skip_to_first_boss { first_boss } else { 0 };
    let mut args = vec![("api_key", api_key.to_string()), ("end", end.to_string())];
    if let Some(name) = actorname {
        // No, despite the sourceID field in the output you can't
        // filter on it (I think source.id is probably guid), and
        // source.name won't match a combatantinfo.
        //
        // actorid does work, but then you can't get encounter info,
        // and I'd have to store that separately or something rather
        // than leaving them in the event stream
        args.push(("filter", format!(r#"type="encounterstart" or type="encounterend" or type="combatantinfo" or source.name = "{}""#, name)));
    }
    base_url = Url::parse_with_params(&base_url, &args)
        .unwrap().into_string();
    Iter {
        intern: intern,
        chunk: Err(start),
        next: 0,
        client: client,
        base_url: base_url,
        names: names,
        print: false,
    }
}

impl<'a> Iter<'a> {
    fn fixup_chunk(&self, json: &mut JsonValue) {
        if json["events"].len() == 0 {
            return;
        }

        // Work around WCL API bugs:
        // 1. If the start time is exactly the timestamp of an encounterend, the encounterend is skipped
        // 2. Sometimes nextPageTimestamp isn't sent
        //
        // So decrement the timestamp (or insert one at the last event) and remove any elements at that timestamp
        let ts = json["nextPageTimestamp"].as_i64().map(|t| t - 1).unwrap_or_else(
            || json["events"].members().rev().nth(0).unwrap()["timestamp"].as_i64().unwrap());

        // We're probably legitimately at the end
        if json["events"][0]["timestamp"] == ts {
            return;
        }

        if json["nextPageTimestamp"].as_i64().is_none() {
            println!("missing");
        }

        if let JsonValue::Array(ref mut array) = json["events"] {
            let mut i = array.len() - 1;

            while array[i]["timestamp"] == ts {
                if array[i]["type"] != "encounterend" {
                    array.remove(i);
                }
                i -= 1;
            }
            
        }
        json["nextPageTimestamp"] = JsonValue::from(ts);
        
    }
    fn load_chunk(&mut self, next_start: u64) {
        let mut json = get_json(&self.client, Url::parse_with_params(&self.base_url, &[("start", &next_start.to_string())]).unwrap());
        self.fixup_chunk(&mut json);

        self.chunk = Ok(json);
        self.next = 0;
    }
    fn parse_object(&self, id: isize) -> Object<'a> {
        let idstr = self.intern.intern(&id.to_string());
        Object {
            id: idstr,
            name: *self.names.get(&id).unwrap_or(&idstr),
        }
    }
    fn parse_base(&self, json: &JsonValue) -> BaseInfo<'a> {
        fn get_id(json: &JsonValue, base: &str, baseid: &str) -> isize {
            if let Some(x) = json[baseid].as_isize() {
                x
            } else {
                // normal ids are 1+. "Environment" will have guid 0
                -json[base]["guid"].as_isize().unwrap()
            }
        }
        BaseInfo {
            timestamp: Duration::milliseconds(json["timestamp"].as_i64().unwrap()),
            // wcl uses a different setup for pets, don't worry about it?
            src: self.parse_object(get_id(json, "source", "sourceID")),
            dst: self.parse_object(get_id(json, "target", "targetID")),
            src_flags1: 0, src_flags2: 0,
            dst_flags1: 0, dst_flags2: 0,
        }
    }
    fn parse_entry(&self, json: &JsonValue) -> Entry<'a> {
        //println!("{}", json);
        let ts = Duration::milliseconds(json["timestamp"].as_i64().unwrap());
        let intern = self.intern;
        let ty = json["type"].as_str().unwrap();
        match ty {
            "encounterstart" =>
                Entry::EncounterStart {
                    ts: ts,
                    name: intern.intern(json["name"].as_str().unwrap()),
                    id: json["encounterID"].as_u32().unwrap(),
                    difficulty: json["difficulty"].as_u16().unwrap(),
                },
            "encounterend" =>
                Entry::EncounterEnd {
                    ts: ts,
                    name: intern.intern(json["name"].as_str().unwrap()),
                    id: json["encounterID"].as_u32().unwrap(),
                    difficulty: json["difficulty"].as_u16().unwrap(),
                    kill: json["kill"] == true,
                },
            "combatantinfo" =>
                Entry::Info {
                    ts: ts,
                    id: intern.intern(&json["sourceID"].to_string()),
                    strength: json["strength"].as_u32().unwrap(),
                    agi: json["agility"].as_u32().unwrap(),
                    sta: json["stamina"].as_u32().unwrap(),
                    int: json["intellect"].as_u32().unwrap(),
                    dodge: json["dodge"].as_u32().unwrap(),
                    parry: json["parry"].as_u32().unwrap(),
                    block: json["block"].as_u32().unwrap(),
                    critm: json["critMelee"].as_u32().unwrap(),
                    critr: json["critRanged"].as_u32().unwrap(),
                    crits: json["critSpell"].as_u32().unwrap(),
                    spd: json["speed"].as_u32().unwrap(),
                    steal: json["leech"].as_u32().unwrap(),
                    hastem: json["hasteMelee"].as_u32().unwrap(),
                    hastr: json["hasteRanged"].as_u32().unwrap(),
                    hastes: json["hasteSpell"].as_u32().unwrap(),
                    avd: json["avoidance"].as_u32().unwrap(),
                    mastery: json["mastery"].as_u32().unwrap(),
                    versm: json["versatilityDamageDone"].as_u32().unwrap(),
                    versr: json["versatilityHealingDone"].as_u32().unwrap(),
                    verss: json["versatilityDamageReduction"].as_u32().unwrap(),
                    armor: json["armor"].as_u32().unwrap(), 
                },
            "heal" => {
                let effective = json["amount"].as_u64().unwrap();
                let overheal = json["overheal"].as_u64().unwrap_or(0);
                Entry::Heal {
                    ty: if json["tick"] == true { HealType::Periodic } else { HealType::Heal },
                    base: self.parse_base(json),
                    id: json["ability"]["guid"].as_u32().unwrap(),
                    spell: intern.intern(json["ability"]["name"].as_str().unwrap()),
                    flags: 0,
                    hp: json["hitPoints"].as_u64().unwrap(),
                    maxhp: json["maxHitPoints"].as_u64().unwrap(),
                    heal: effective + overheal,
                    overheal: overheal,
                    crit: json["hitType"] == 2
                }
            },
            "applybuff" | "removebuff" | "refreshbuff" | "applybuffstack" | "removebuffstack" |
            "applydebuff" | "removedebuff" | "refreshdebuff" | "applydebuffstack" | "removedebuffstack" => {
                let buff = !ty.contains("debuff");
                let ty = if ty.ends_with("stack") {
                    AuraType::Stack
                } else if ty.starts_with("apply") {
                    AuraType::Apply
                } else if ty.starts_with("remove") {
                    AuraType::Remove
                } else {
                    AuraType::Refresh
                };
                Entry::Aura {
                    ty: ty,
                    base: self.parse_base(json),
                    id: json["ability"]["guid"].as_u32().unwrap(),
                    aura: intern.intern(json["ability"]["name"].as_str().unwrap()),
                    flags: 0,
                    buff: buff,
                }
            }
            _ => Entry::Unknown(ts, intern.intern(&json.to_string()))
        }
    }
    

}



impl<'a> Iterator for Iter<'a> {
    type Item = Entry<'a>;
    fn next(&mut self) -> Option<Entry<'a>> {
        if let Err(next) = self.chunk {
            self.load_chunk(next);
        }
        let chunk = ::std::mem::replace(&mut self.chunk, Err(0)).unwrap();
        let len = chunk["events"].len();
        if len == 0 {
            return None;
        }
        let ret = self.parse_entry(&chunk["events"][self.next]);
        self.next += 1;
        if self.next < len {
            self.chunk = Ok(chunk);
        } else {
            self.chunk = Err(chunk["nextPageTimestamp"].as_u64().unwrap());
        }
        Some(ret)
    }
}

