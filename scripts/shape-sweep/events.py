"""Add untimed path counters to the disposable test build only."""
import difflib
from pathlib import Path
import sys


def install(root):
    root=Path(root)
    additions={
        "src/query/engine/mod.rs": '''
#[cfg(test)]
thread_local! {
    pub(crate) static SWEEP_EVENTS: std::cell::RefCell<std::collections::BTreeMap<&'static str,u64>> = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}
#[cfg(test)]
pub(crate) fn sweep_event(name: &'static str) {
    SWEEP_EVENTS.with(|s| *s.borrow_mut().entry(name).or_default() += 1);
}
''',
    }
    replacements={
        "src/query/engine/join.rs": [
            ("entry.used = p.tick;", 'entry.used = p.tick;\n                #[cfg(test)] super::sweep_event("entity_probe_cache_hit");'),
            ("self.inner.probe(uuid, eid);", 'self.inner.probe(uuid, eid);\n        #[cfg(test)] super::sweep_event("entity_probe_open");'),
        ],
        "src/query/engine/recursive.rs": [
            ("Ok(alternative)", '{ #[cfg(test)] super::sweep_event("ancestor_alternative_selected"); Ok(alternative) }'),
        ],
        "src/query/engine/scan.rs": [
            ("ranges.push((Index::Avet, keys::attribute_value_prefix(a, &kv)));",
             'ranges.push((Index::Avet, keys::attribute_value_prefix(a, &kv)));\n                    #[cfg(test)] super::sweep_event("bound_value_avet_range");'),
        ],
        "src/query/datalog/optimizer.rs": [
            ("let next = patterns.remove(best.map_or(0, |(i, _)| i));",
             'let at = best.map_or(0, |(i, _)| i);\n        #[cfg(test)] { crate::query::engine::sweep_event("connected_delta_step"); if at != 0 { crate::query::engine::sweep_event("delta_dynamic_reprioritization"); } }\n        let next = patterns.remove(at);'),
        ],
        "src/query/engine/ids.rs": [
            ("return Ok(*u);", '#[cfg(test)] super::sweep_event("uuid_cache_hit");\n            return Ok(*u);'),
            ("Some(u) => u,\n            None => k.uuid_of(eid)?,",
             'Some(u) => { #[cfg(test)] super::sweep_event("uuid_hint_hit"); u },\n            None => { #[cfg(test)] super::sweep_event("uuid_scalar_miss"); k.uuid_of(eid)? },'),
        ],
        "src/storage/dict.rs": [
            ("if chunk.len() < 64 || last.saturating_sub(first) > span_limit {",
             'if chunk.len() < 64 || last.saturating_sub(first) > span_limit {\n                #[cfg(test)] crate::query::engine::sweep_event("uuid_batch_scalar");'),
            ("let start = keys::dict_eid_key(first);",
             '#[cfg(test)] crate::query::engine::sweep_event("uuid_batch_dense");\n            let start = keys::dict_eid_key(first);'),
        ],
    }
    patch=[]
    for name in additions.keys() | replacements.keys():
        path=root/name
        original=path.read_text()
        text=original+additions.get(name,"")
        for old,new in replacements.get(name,[]):
            if text.count(old)!=1:
                raise ValueError(f"missing/ambiguous event site: {name}: {old}")
            text=text.replace(old,new)
        path.write_text(text)
        patch.extend(difflib.unified_diff(original.splitlines(True),text.splitlines(True),fromfile="a/"+name,tofile="b/"+name))
    (root/"evidence/events.patch").write_text("".join(patch))


if __name__=="__main__":
    install(sys.argv[1])
