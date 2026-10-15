# FolderSize

A free disk space analyzer for Windows. It scans a drive or folder and shows how much space every folder and file takes, so you can find what's filling up your SSD and clean it up.

No install, no account, no trial. One `.exe`.

## What it does

- **Fast parallel scan.** All CPU cores walk the tree at once. A 1.7 TB drive with 1.6 million files scans in about 30 seconds the first time, and in a few seconds once Windows has the folders cached.
- **Folder tree** sorted biggest first, with size, a % of parent bar, file and folder counts, and last-modified date.
- **Treemap** of the selected folder: every box is a folder or file, sized by how much space it takes. Click a box to find it in the tree. Double-click a folder to zoom in, and use **Up** to zoom out.
- **Largest files** tab: the 500 biggest files on the scanned drive or folder.
- **Cleanup**: right-click anything to *Show in Explorer*, *Copy path* or *Send to Recycle Bin*. It asks you first, and it never deletes anything permanently.
- **Export CSV** of every folder with its size, for spreadsheets.
- **Drive bar** at the top shows free space on every drive. Click a drive to scan it.

## Usage

Download `foldersize.exe` from Releases (or build it, see below) and run it. Click a drive, or **Folder…** to pick one.

You can also pass a path: `foldersize.exe D:\Games`.

Some system folders can only be read by an administrator. Click **Run as administrator** in the bottom-right to rescan with them included. Folders that couldn't be read are marked with ⚠.

### How sizes are counted

- **Size** is the files' real size. Hover over a size to also see the estimated **on disk** size.
- OneDrive files that are only in the cloud count as 0 on disk.
- Junctions and symbolic links are shown but not followed, so nothing is counted twice through a link.
- Files with several hard links (common in `C:\Windows\WinSxS` and some mod managers) are counted once per link. So a whole-drive total can come out a bit higher than the drive's used space.
- Moving something to the Recycle Bin does not free space until you empty the bin. `$Recycle.Bin` shows up in the tree too, often as a big folder.

## Building

Needs [Rust](https://rustup.rs) (stable).

```
cargo build --release
```

The exe is at `target\release\foldersize.exe`.

## License

MIT
