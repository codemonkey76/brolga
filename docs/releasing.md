# Releasing Brolga

For the maintainer. How a version goes out, and how the key that lets
installed copies trust it is kept.

## Making a release

1. Put the new version in the workspace `Cargo.toml` (and so `Cargo.lock`),
   `app/package.json` and the example in `docs/control-protocol.md`, and add a
   `<release>` saying what has changed to `packaging/linux/io.github.codemonkey76.Brolga.metainfo.xml`. Commit it as
   `Brolga X.Y.Z: …`, saying the same in words.
2. Push a tag for it: `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. The **Release** workflow builds every package (Linux: deb, rpm, AppImage
   and Flatpak; Windows: installer and MSI; macOS: app and disk image), signs
   each one an installed copy can update itself with, writes `latest.json`
   and `SHA256SUMS`, and makes a **draft** release.
4. Look the draft over and publish it. Installed copies look for
   `latest.json` on the newest published release, so nothing is offered to
   anyone until then.

## The update key

An installed Brolga replaces itself only with a package signed by Brolga's
own update key. Its public half is built into every copy
(`plugins.updater.pubkey` in `tauri.conf.json`); whoever has the private
half can publish an update every copy will take. It is kept in three places:

- the `TAURI_SIGNING_PRIVATE_KEY` secret of this repository, which the
  Release workflow signs with;
- the maintainer's computer, readable by that user only, on an encrypted
  disk;
- the maintainer's password manager, as the backup. GitHub never shows a
  secret again once it is saved, so this is the copy to restore from.

**It has no password, by decision** (TKT-01061). The workflow would need the
password as a second secret beside the key, so it would add nothing for
anyone who reached the secrets, and the other copies are already behind an
encrypted disk and the password manager.

**It is not replaced while it is safe.** Every copy trusts this one public key
and finds updates through one `latest.json`, so a new key cannot simply take
its place: see below.

### If the key is lost

If every copy is lost, no update can reach an installed Brolga again.
Restore the secret from the password manager first. Failing that, make a new
key and tell everyone to install the next version by hand from the releases
page; installed copies will not take it themselves.

### If the key may have leaked

Change it, in this order:

1. Make a new key: `bunx tauri signer generate -w brolga-updater.key`
   (in `app/`). Keep it as the old one was kept.
2. Put its public half in `tauri.conf.json` and release that version
   **signed with the old key**. This is the bridge: a copy that takes it
   trusts the new key from then on.
3. Replace the `TAURI_SIGNING_PRIVATE_KEY` secret with the new key, and sign
   every release after the bridge with it.
4. A copy that misses the bridge (off for weeks, say) will see the next
   release, find it signed with a key it does not trust, and refuse it. Its
   owner has to install a new version by hand. Say so in the release notes,
   and leave the bridge release in place.

Until the bridge is out, whoever has the old key can still publish an update
that copies will take, so move quickly, and remove the old key from every
place it was kept once the bridge is published.
