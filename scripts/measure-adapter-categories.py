#!/usr/bin/env python3
"""Classify an adapter's production lines by what kind of work they do.

Phase U2's central measurement. Categories follow the phase brief:

  A interface/instruction decoding     discriminators, arity, argument reads
  B account-role binding               positions to roles, label assignment
  C universal evidence plumbing        calls into evidence/ and standard_programs/
  D protocol invariants                admission checks, corroboration
  E protocol-specific math             arithmetic only this protocol defines
  F finding/semantic interpretation    subjects, findings, summaries, features
  G custom evaluator                   named, versioned protocol conversions
  -  types, constants, documentation

Assignment is by enclosing item (function / const / struct), declared in the
TABLE below per file, so the classification is auditable rather than a guess at
what a line "looks like".
"""
import re, sys, collections, importlib.util

spec = importlib.util.spec_from_file_location("d", "scripts/measure-adapter-duplication.py")
d = importlib.util.module_from_spec(spec); spec.loader.exec_module(d)

def items(path):
    """(name, kind, [lines]) for each top-level-ish item, production only."""
    cut = d.in_tests(path)
    src = open(path).read().split("\n")[:cut]
    out, i = [], 0
    pat = re.compile(r"\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+fn\s+|fn\s+|const\s+|static\s+|struct\s+|enum\s+|impl\b|type\s+)")
    name_pat = re.compile(r"\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+fn\s+|fn\s+|const\s+|static\s+|struct\s+|enum\s+|type\s+)([A-Za-z_0-9]+)")
    while i < len(src):
        line = src[i]
        if pat.match(line) and not line.strip().startswith("//"):
            m = name_pat.match(line)
            name = m.group(1) if m else "impl"
            depth, started, body, j = 0, False, [], i
            while j < len(src):
                for ch in src[j]:
                    if ch == "{": depth += 1; started = True
                    elif ch == "}": depth -= 1
                body.append(src[j]); j += 1
                if started and depth <= 0: break
                if not started and src[j-1].rstrip().endswith(";"): break
            if name == "impl":
                # Recurse one level: an impl block's methods are classified
                # individually, which is where the interesting distinctions are.
                inner, k = [], 1
                while k < len(body) - 1:
                    if re.match(r"\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+", body[k]):
                        m2 = re.match(r"\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+([A-Za-z_0-9]+)", body[k])
                        dep2, st2, b2, l = 0, False, [], k
                        while l < len(body) - 1:
                            for ch in body[l]:
                                if ch == "{": dep2 += 1; st2 = True
                                elif ch == "}": dep2 -= 1
                            b2.append(body[l]); l += 1
                            if st2 and dep2 <= 0: break
                        inner.append((m2.group(1), b2)); k = l
                    else: k += 1
                out.extend(inner)
            else:
                out.append((name, body))
            i = j; continue
        i += 1
    return out

# name -> category. Anything unlisted is "-" (types, constants, docs).
TABLE = {
 "engine/src/protocol/kamino/mod.rs": {
   "A": ["from_discriminator","name","is_deposit","has_farms","account_count","recognises",
         "requested_amount","klend_discriminators","operation"],
   "B": ["role","roles","role_address","labels","label","reserve_label","account",
         "role_reserve_address","operation_from_accounts","economic_entity_id",
         "required_accounts"],
   "C": ["token_balance","mint_supply","token_moved","interpret","prove_boundaries",
         "any_token_program_amount","any_token_program_mint","obligation_positions",
         "reserve_of","obligation_of","liquidity_decimals"],
   "D": ["accept","cpi_programs","dependency_programs","supports_cpi"],
   "E": ["boundaries"],
   "F": ["summarize","named_findings","evaluable_subjects","decoded_sources_of",
         "decoded_byte_ranges","state_features","decode","decode_reserve_fields",
         "decode_obligation_fields","action_id","semantic_action","protocol_id",
         "quantity_field","adapter_version","program_id"],
 },
 "engine/src/protocol/kamino/state.rs": {
   "A": ["anchor_discriminator"],
   "F": ["decode_reserve","decode_obligation","is_lending_market","borrow_against","collateral_in"],
 },
 "engine/src/protocol/kamino/fraction.rs": {
   "G": ["to_base_units","fractional_remainder","delta_base_units","delta_scaled"],
 },
 "engine/src/protocol/stake_pool.rs": {
   "A": ["from_discriminant","name","data_len","operation","operation_amount","deposit",
         "deposit_lamports","declared_programs"],
   "B": ["roles","authority_position","entity_position","labels","label","after",
         "economic_entity_id","required_accounts","pool_decimals"],
   "C": ["interpret","prove_boundaries","rescale"],
   "D": ["accept","cpi_programs","dependency_programs","supports_cpi"],
   "E": ["apply","lamports_for_withdrawal","pool_tokens_for_deposit","basis_points","boundaries"],
   "F": ["summarize","named_findings","evaluable_subjects","decoded_sources_of",
         "decoded_byte_ranges","state_features","decode","action_id","semantic_action",
         "adapter_version","program_id"],
   "G": [],
 },
}

def classify(path):
    table = TABLE.get(path, {})
    lookup = {}
    for cat, names in table.items():
        for n in names: lookup[n] = cat
    counts = collections.Counter()
    detail = collections.defaultdict(list)
    for name, body in items(path):
        n = len(d.strip(body))
        cat = lookup.get(name, "-")
        counts[cat] += n
        detail[cat].append((name, n))
    return counts, detail

LABELS = {
 "A":"A interface/instruction decoding","B":"B account-role binding",
 "C":"C universal evidence plumbing","D":"D protocol invariants",
 "E":"E protocol-specific math","F":"F finding/semantic interpretation",
 "G":"G custom evaluator","-":"- types, constants, docs",
}

for path in sys.argv[1:]:
    counts, detail = classify(path)
    total = sum(counts.values())
    print(f"\n== {path} == {total} production lines")
    for cat in ["A","B","C","D","E","F","G","-"]:
        n = counts.get(cat, 0)
        if not n and cat not in LABELS: continue
        pct = (100*n//total) if total else 0
        print(f"  {LABELS[cat]:<38}{n:>6}{pct:>5}%")
        if "-v" in sys.argv:
            for nm, c in sorted(detail[cat], key=lambda x:-x[1])[:8]:
                print(f"        {nm:<40}{c:>5}")
