# Printer setup help (pending)

Goal: get a printer from the box to the point where PrinterTUI can use it.

## Findings

- Most printers from the last ~10 years (HP, Epson, Canon, Brother, Xerox, Kyocera, Ricoh, Lexmark, Samsung, Konica Minolta) work without drivers (IPP Everywhere, AirPrint, Mopria). PrinterTUI already adds them that way.
- What usually blocks people: getting the printer on Wi-Fi, enabling IPP / AirPrint / eSCL in its web page, and USB-only printers that need a driver.
- Downloading manufacturers' manuals is not an option: they are copyrighted (cannot ship in the repo), large, and go stale.

## Plan

1. Research, delegated to the Antigravity CLI (`agy`, through tmux): per brand, how to join Wi-Fi, how to enable IPP / AirPrint / eSCL, which lines are not driverless. Output: a report with sources, not copies of manuals.
2. A "printer doctor" in the app: works out the situation (not found on the network, found without IPP, USB without a driver, offline...) and shows short steps for that brand, written by us from the report.
3. Maybe later: an "Ask AI" button that only explains, using the doctor's findings. It would not run commands (`sudo lpadmin` from a model is too risky) and needs an API key.

## Not doing

- An AI that configures the printer by itself.
- Shipping manufacturers' PDFs.
