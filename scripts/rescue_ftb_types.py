# -*- coding: utf-8 -*-
"""Rescue FTB Quests structural fields corrupted by translation (CJK type/shape/auto)."""
from __future__ import annotations

import re
import shutil
from collections import Counter
from pathlib import Path

# Known Chinese -> English for FTB structural enums (from CTE2 corruption + vanilla FTB types)
TYPE_MAP = {
    "勾選標記": "checkmark",
    "物品": "item",
    "指令": "command",
    "擊殺": "kill",
    "擊殺實體": "kill",
    "維度": "dimension",
    "生態域": "biome",
    "位置": "location",
    "經驗": "xp",
    "觀察": "observation",
    "檢查點": "checkmark",
    "自訂": "custom",
    "階段": "stage",
    "進度": "advancement",
    "統計": "stat",
    "屬性": "stat",
    "結構": "structure",
    "選擇": "choice",
    "戰利品": "loot",
    "全部表格": "all_table",
    "所有表格": "all_table",
    "流體": "fluid",
    "能量": "forge_energy",
    "科技": "tech",
}
SHAPE_MAP = {
    "圓圈": "circle",
    "方塊": "square",
    "方形": "square",
    "菱形": "diamond",
    "鑽石": "diamond",
    "五邊形": "pentagon",
    "六邊形": "hexagon",
    "八邊形": "octagon",
    "心形": "heart",
    "齒輪": "gear",
    "裝備": "gear",
    "無": "none",
    "圓角方塊": "rsquare",
}
AUTO_MAP = {
    "已啟用": "enabled",
    "啟用": "enabled",
    "停用": "disabled",
    "已停用": "disabled",
    "隱形": "invisible",
    "無": "no",
}

STRUCT_KEYS = {
    "type": TYPE_MAP,
    "shape": SHAPE_MAP,
    "auto": AUTO_MAP,
    "default_quest_shape": SHAPE_MAP,
}


def has_cjk(s: str) -> bool:
    return any("\u4e00" <= c <= "\u9fff" for c in s)


def rescue_text(text: str) -> tuple[str, Counter]:
    stats: Counter = Counter()

    def repl(m: re.Match) -> str:
        key, val = m.group(1), m.group(2)
        mapping = STRUCT_KEYS.get(key)
        if not mapping or not has_cjk(val):
            return m.group(0)
        eng = mapping.get(val)
        if eng is None:
            stats[f"UNMAPPED:{key}:{val}"] += 1
            return m.group(0)
        stats[f"{key}:{val}->{eng}"] += 1
        return f'{key}: "{eng}"'

    # key: "value"
    out = re.sub(
        r'\b(type|shape|auto|default_quest_shape):\s*"([^"]*)"',
        repl,
        text,
    )
    return out, stats


def collect_unmapped(root: Path) -> Counter:
    c: Counter = Counter()
    for p in root.rglob("*.snbt"):
        t = p.read_text(encoding="utf-8", errors="replace")
        for key, mapping in STRUCT_KEYS.items():
            for m in re.finditer(rf'\b{key}:\s*"([^"]*)"', t):
                v = m.group(1)
                if has_cjk(v) and v not in mapping:
                    c[f"{key}:{v}"] += 1
    return c


def rescue_tree(src: Path, dry_run: bool = False) -> Counter:
    total: Counter = Counter()
    for p in src.rglob("*.snbt"):
        text = p.read_text(encoding="utf-8", errors="replace")
        new, stats = rescue_text(text)
        if stats and new != text:
            total.update(stats)
            if not dry_run:
                bak = p.with_suffix(p.suffix + ".pre-rescue.bak")
                if not bak.exists():
                    shutil.copy2(p, bak)
                p.write_text(new, encoding="utf-8", newline="\n")
    return total


def dump_remaining(root: Path, out: Path) -> None:
    c: Counter = Counter()
    for p in root.rglob("*.snbt"):
        t = p.read_text(encoding="utf-8", errors="replace")
        for key in STRUCT_KEYS:
            for m in re.finditer(rf'\b{key}:\s*"([^"]*)"', t):
                v = m.group(1)
                if has_cjk(v):
                    c[f"{key}\t{v}"] += 1
    out.write_text(
        "\n".join(f"{n}\t{k}" for k, n in c.most_common()) + "\n",
        encoding="utf-8",
    )


def main() -> None:
    roots = [
        Path(
            r"C:/Users/jolin/AppData/Roaming/PrismLauncher/instances/Craft to Exile 2 TEST/minecraft/config/ftbquests"
        ),
        Path(
            r"C:/Users/jolin/AppData/Roaming/PrismLauncher/instances/Craft to Exile 2 TEST/翻譯結果/config/ftbquests"
        ),
    ]
    for root in roots:
        if not root.is_dir():
            print("MISSING", root)
            continue
        unmapped = collect_unmapped(root)
        print("===", root)
        print("unmapped before count:", len(unmapped))
        stats = rescue_tree(root, dry_run=False)
        print("rescued replacements:", sum(v for k, v in stats.items() if not k.startswith("UNMAPPED")))
        print("unmapped attempts:", sum(v for k, v in stats.items() if k.startswith("UNMAPPED")))
        dump = Path(__file__).with_name("_unmapped_ftb.txt")
        dump_remaining(root, dump)
        print("remaining dump:", dump, "lines", len(dump.read_text(encoding="utf-8").splitlines()))


if __name__ == "__main__":
    main()
