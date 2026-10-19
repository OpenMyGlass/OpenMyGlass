#!/usr/bin/env python3
"""Moves the Android emulator's GPS position along a route, so navigation apps advance as if driving.

Usage: simulate-route.py ROUTE.csv [--speed-kmh 40] [--interval 1.0] [--adb PATH] [--serial emulator-5554]
ROUTE.csv: one "lat,lon" per line (comments with #). Points are interpolated at the given speed and
sent with `adb emu geo fix <lon> <lat>` once per interval.
"""
import argparse
import math
import os
import subprocess
import sys
import time

EARTH_RADIUS_M = 6_371_000


def load_route(path):
    points = []
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            lat, lon = (float(x) for x in line.split(","))
            points.append((lat, lon))
    if len(points) < 2:
        raise SystemExit("route needs at least two points")
    return points


def distance_m(a, b):
    lat1, lon1, lat2, lon2 = map(math.radians, (*a, *b))
    h = math.sin((lat2 - lat1) / 2) ** 2 + math.cos(lat1) * math.cos(lat2) * math.sin((lon2 - lon1) / 2) ** 2
    return 2 * EARTH_RADIUS_M * math.asin(math.sqrt(h))


def interpolate(points, step_m):
    """Yields positions every step_m metres along the polyline (linear interpolation per segment)."""
    yield points[0]
    carry = 0.0
    for a, b in zip(points, points[1:]):
        seg = distance_m(a, b)
        pos = step_m - carry
        while pos <= seg:
            t = pos / seg
            yield (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
            pos += step_m
        carry = seg - (pos - step_m)
    yield points[-1]


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("route")
    ap.add_argument("--speed-kmh", type=float, default=40.0)
    ap.add_argument("--interval", type=float, default=1.0)
    ap.add_argument("--adb", default=os.path.expanduser("~/Library/Android/sdk/platform-tools/adb"))
    ap.add_argument("--serial", default=None)
    ap.add_argument("--dry-run", action="store_true", help="print positions instead of sending them")
    args = ap.parse_args()

    step = args.speed_kmh / 3.6 * args.interval
    base = [args.adb] + (["-s", args.serial] if args.serial else [])
    for lat, lon in interpolate(load_route(args.route), step):
        if args.dry_run:
            print(f"{lat:.6f},{lon:.6f}")
            continue
        subprocess.run(base + ["emu", "geo", "fix", f"{lon:.6f}", f"{lat:.6f}"], check=True,
                       stdout=subprocess.DEVNULL)
        time.sleep(args.interval)


if __name__ == "__main__":
    sys.exit(main())
