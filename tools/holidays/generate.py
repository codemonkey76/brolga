#!/usr/bin/env python3
"""Write the public holidays Brolga carries, from the `holidays` library.

    python3 -m venv /tmp/holidays && /tmp/holidays/bin/pip install holidays==0.106
    /tmp/holidays/bin/python tools/holidays/generate.py FIRST_YEAR LAST_YEAR

The library (https://github.com/vacanza/holidays, MIT) is not shipped: only
the dates and names it works out, as crates/calendar/data/holidays.json.
A country's days are those kept by the whole of it. A region has the days
it adds to those, and the days it does not keep.
"""

import json
import pathlib
import re
import sys

import holidays

OUT = pathlib.Path(__file__).resolve().parents[2] / "crates/calendar/data/holidays.json"


# The library's name for each country, by its two-letter code.
NAMES = {each[1]: each[0] for each in holidays.registry.COUNTRIES.values()}


def language(kind):
    """English where the library has it, and otherwise the country's own."""
    own = kind.default_language
    known = kind.supported_languages or ()
    if own and own.startswith("en"):
        return own
    return "en_US" if "en_US" in known else own


def days(code, years, region=None):
    kind = holidays.country_holidays(code).__class__
    found = holidays.country_holidays(code, subdiv=region, years=years, language=language(kind))
    return {day.isoformat(): name for day, name in found.items()}


def spaced(name):
    return re.sub(r"(?<=[a-z])(?=[A-Z])", " ", name)


def main():
    first, last = int(sys.argv[1]), int(sys.argv[2])
    years = range(first, last + 1)
    countries = []
    for code, regions in sorted(holidays.list_supported_countries(include_aliases=False).items()):
        kind = holidays.country_holidays(code).__class__
        names = {short: long for long, short in (getattr(kind, "subdivisions_aliases", None) or {}).items()}
        whole = days(code, years)
        country = {"code": code, "name": spaced(NAMES.get(code, code)), "days": sorted(whole.items()), "regions": []}
        for region in regions:
            theirs = days(code, years, region)
            country["regions"].append(
                {
                    "code": region,
                    "name": names.get(region, region),
                    "add": sorted((day, name) for day, name in theirs.items() if whole.get(day) != name),
                    "drop": sorted(day for day in whole if day not in theirs),
                }
            )
        if country["days"] or any(region["add"] for region in country["regions"]):
            countries.append(country)

    made = {"source": f"holidays {holidays.__version__} (https://github.com/vacanza/holidays, MIT)", "first_year": first, "last_year": last, "countries": countries}
    OUT.write_text(json.dumps(made, ensure_ascii=False, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"{len(countries)} countries, {sum(len(c['regions']) for c in countries)} regions, {OUT.stat().st_size:,} bytes")


if __name__ == "__main__":
    main()
