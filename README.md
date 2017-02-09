Wow combat log parser
=====================

This is a library to parse World of Warcraft combat logs in rust, and
the primary use I wrote it for: an analyzer for resto druid mastery.

Compilation and running
-----------------------

Install rust `https://www.rust-lang.org/en-US/install.html`

```bash
cargo build
./target/debug/resto_druid_mastery /path/to/wow/Logs/WoWCombatLog.txt MyCharacter-MyRealm [start] [end]
```

`start` and `end` are optional and are measured in seconds from the
start of the log (start and length values are printed for each boss
in the log).

How the analyzer works
----------------------

This analyzer works by computing the weighted average of effective
healing done by the number of mastery stacks on the target at the time
of healing, using the amount of healing that would have been done if
there was no mastery, and only including healing that is affected by
mastery at all. This is the appropriate value to use for computation
of mastery stat weights.

This analyzer also provides some other values that are useful for
determining stat weights or the benefit of gear/talents (some of which
are also easily determined from warcraftlogs.com):

scale_mastery_frac: the average number of stacks of mastery (weighted by no-mastery healing done)
scale_living_seed: the fraction of heals affected by LS (normalized as if no heals crit)
scale_regrowth: the fraction of healing from regrowth (normalized as if no heals crit)
scale_tranq: the fraction of healing from tranquility
scale_rejuv: the fraction of healing from rejuv

scale_2pc: the fraction of your healing that benefited from the 2pc
scale_2pc_added: the fraction of your healing that was provided by the 2pc
scale_cult: the fraction of your healing that benefited from cultivation

Weaknesses
----------

To compute these values the parser needs to know your current mastery
at all times. Wow provides the mastery value at the start of each boss
encounter, but not changes due to procs during a fight. This analyzer
handles the Astral Warden 2pc set bonus correctly, but does not handle
any trinket procs, which will result in slightly overvaluing mastery
during said procs. Missing the 2pc bonus resulted in less than a 2%
incorrect increase in mastery's weight, so I'm not too worried.

In addition, this tool assumes that healing trinkets/enchants that
scale with some of int/vers/haste/crit, but not mastery are handled
elsewhere (Torty's spreadsheet does this for ancient priestess), and
ignores their healing for mastery computation. Ysera's gift and prydaz
are correctly handled this way, unless you're actually comparing your
stamina stat weight. It also ignores leech, because I have not tested
its scaling, and it would be a complete pain to deal with.

The analyzer assumes that the number of stacks of mastery when a
living seed goes off is the same as the number that existed when it
was created. The alternative would be to correctly measure that, but
have no idea about its overheal (or being replaced/falling off)
amounts, and having to parse the number of ranks of +Living seed you
have.
