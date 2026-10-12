# Data

`windowsZones.xml` is the Unicode CLDR table of Windows time zone names and
the IANA zones they mean, exactly as published at
<https://github.com/unicode-org/cldr/blob/main/common/supplemental/windowsZones.xml>.
It is © Unicode, Inc. and used under the Unicode License v3
(<https://www.unicode.org/license.txt>).

To take up a newer table, replace the file. Nothing else changes: a test
checks that every zone it names is one the built-in zone database has.

`holidays.json` is the public holidays of each country and its regions for a
span of years, written by `tools/holidays/generate.py` from the `holidays`
library (<https://github.com/vacanza/holidays>, MIT). The library is not
shipped, only the dates and names it works out. To take up later years or a
newer release of it, run the script again as its first lines say.
