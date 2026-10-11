# Data

`windowsZones.xml` is the Unicode CLDR table of Windows time zone names and
the IANA zones they mean, exactly as published at
<https://github.com/unicode-org/cldr/blob/main/common/supplemental/windowsZones.xml>.
It is © Unicode, Inc. and used under the Unicode License v3
(<https://www.unicode.org/license.txt>).

To take up a newer table, replace the file. Nothing else changes: a test
checks that every zone it names is one the built-in zone database has.
