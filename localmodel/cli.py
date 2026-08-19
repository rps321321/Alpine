from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .config import REPO_ROOT, artifact_manifest, install_session, powershell, profiles, read_json, sha256
from .agentbench import _parse_events, run_agentbenchmark
from .contextbench import run_contextbenchmark
from .microbench import run_microbenchmark, summarize
from .report import comparison_markdown, latest_profile_rows, write_comparison
from .qualification import qualify_run_row
from .store import ResultStore


def default_install_root() -> Path:
    return Path.home() / "local-models"


def run(command: list[str], check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, text=True, check=False)
    if check and result.returncode != 0:
        raise SystemExit(result.returncode)
    return result


def cmd_profiles(_: argparse.Namespace) -> int:
    print("PROFILE              STATUS                 CONTEXT  OUTPUT  CPU FFN  SPECULATION")
    for name, profile in profiles().items():
        speculation = f"MTP{profile['mtp_depth']}"
        if profile.get("ngram_mod"):
            speculation += "+ngram(request-local)" if profile.get("ngram_reset_on_begin") else "+ngram(shared)"
        print(f"{name:20} {profile['status']:22} {profile['context']:7} {profile['output']:7} 0-{profile['tensor_cpu_through_block']:<5} {speculation}")
    return 0


def cmd_doctor(args: argparse.Namespace) -> int:
    root = args.install_root.resolve()
    problems: list[str] = []
    print(f"repo: {REPO_ROOT}")
    print(f"install: {root}")
    try:
        session = install_session(root)
    except (OSError, json.JSONDecodeError) as exc:
        print(f"FAIL session config: {exc}")
        return 1
    manifest = artifact_manifest()
    checks = [
        ("model", Path(session["model"]), manifest["model"]),
        ("chat_template", Path(session["chat_template"]), manifest["chat_template"]),
    ]
    if Path(session.get("mmproj", "")).exists():
        checks.append(("mmproj", Path(session["mmproj"]), manifest["mmproj"]))
    for label, path, expected in checks:
        if not path.is_file():
            problems.append(f"{label} missing: {path}")
            continue
        if path.stat().st_size != int(expected["bytes"]):
            problems.append(f"{label} size mismatch: {path}")
            continue
        if args.deep and sha256(path) != expected["sha256"]:
            problems.append(f"{label} hash mismatch: {path}")
            continue
        print(f"OK {label}: {path} ({'hash' if args.deep else 'size'} verified)")
    active_profile = read_json(root / "profiles" / f"{session['active_profile']}.json")
    server_value = session.get("runtimes", {}).get(active_profile.get("runtime")) or session["llama_server"]
    server = Path(server_value)
    if not server.is_file():
        problems.append(f"llama-server missing: {server}")
    else:
        version = subprocess.run([str(server), "--version"], capture_output=True, text=True, check=False)
        identity = (version.stdout + version.stderr).strip().replace("\n", " | ")
        print(f"OK backend: {identity}")
        if "3cb7ffb" not in identity:
            problems.append("backend is not pinned to llama.cpp commit 3cb7ffb")
    opencode = shutil.which("opencode")
    if not opencode:
        problems.append("opencode not found on PATH")
    else:
        version = subprocess.run([opencode, "--version"], capture_output=True, text=True, check=False)
        print(f"OK OpenCode: {version.stdout.strip()}")
        if version.stdout.strip() != "1.18.18":
            problems.append("OpenCode version differs from pinned 1.18.18")
    for problem in problems:
        print(f"FAIL {problem}")
    return 1 if problems else 0


def cmd_inventory(args: argparse.Namespace) -> int:
    output = args.output or REPO_ROOT / "inventory" / f"hardware-{datetime.now(timezone.utc).date().isoformat()}.json"
    command = [
        powershell(), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
        str(REPO_ROOT / "scripts" / "collect-hardware.ps1"),
        "-InstallRoot", str(args.install_root), "-Output", str(output),
    ]
    return run(command, check=False).returncode


def cmd_benchmark(args: argparse.Namespace) -> int:
    run_id, summary = run_microbenchmark(
        args.install_root, args.profile, runs=args.runs, warmups=args.warmups,
        selected_workloads=args.workload or None, keep_server=args.keep_server, notes=args.notes,
    )
    print(f"run_id={run_id}")
    if not args.no_summary:
        print(json.dumps(summary, indent=2))
    return 0 if summary["all_quality_pass"] else 2


def cmd_context_stress(args: argparse.Namespace) -> int:
    run_id, summary = run_contextbenchmark(
        args.install_root, args.profile, ratio=args.ratio, runs=args.runs,
        warmups=args.warmups, keep_server=args.keep_server, notes=args.notes,
    )
    print(f"run_id={run_id}")
    print(json.dumps(summary, indent=2))
    return 0 if summary["all_quality_pass"] else 2


def cmd_agent_benchmark(args: argparse.Namespace) -> int:
    run_id, summary = run_agentbenchmark(
        args.install_root, args.profile, args.task, keep_server=args.keep_server, notes=args.notes,
    )
    print(f"run_id={run_id}")
    print(json.dumps(summary, indent=2))
    return 0 if summary["success"] else 2


def cmd_compare(args: argparse.Namespace) -> int:
    store = ResultStore(REPO_ROOT / "results" / "results.sqlite3")
    try:
        rows = latest_profile_rows(store, args.profiles)
    finally:
        store.close()
    print(comparison_markdown(rows))
    return 0


def cmd_report(args: argparse.Namespace) -> int:
    output = args.output or REPO_ROOT / "reports" / "latest-profile-comparison.md"
    write_comparison(REPO_ROOT / "results" / "results.sqlite3", args.profiles, output)
    print(output)
    return 0


def cmd_runs(args: argparse.Namespace) -> int:
    unknown = sorted(set(args.profiles) - set(profiles()))
    if unknown:
        raise SystemExit(f"unknown profile(s): {', '.join(unknown)}")
    store = ResultStore(REPO_ROOT / "results" / "results.sqlite3")
    try:
        rows = store.runs(args.profiles or None)
    finally:
        store.close()
    print("RUN ID                              PROFILE              KIND       STATUS          STARTED")
    for row in rows[: args.limit]:
        print(f"{row['id']:35} {row['profile']:20} {row['kind']:10} {row['status']:15} {row['started_at']}")
    return 0


def cmd_qualify(args: argparse.Namespace) -> int:
    store = ResultStore(REPO_ROOT / "results" / "results.sqlite3")
    try:
        result = qualify_run_row(store.run(args.run_id), args.target)
    finally:
        store.close()
    print(json.dumps(result, indent=2))
    return 0 if result["promotion_ready"] else 2


def cmd_status(args: argparse.Namespace) -> int:
    return run([
        powershell(), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
        str(installed_script(args.install_root, "show-status.ps1")),
    ], check=False).returncode


def cmd_reconcile(args: argparse.Namespace) -> int:
    store = ResultStore(REPO_ROOT / "results" / "results.sqlite3")
    try:
        row = store.run(args.run_id)
        if not row:
            raise SystemExit(f"run not found: {args.run_id}")
        raw_dir = REPO_ROOT / "results" / "runs" / args.run_id
        samples_path = raw_dir / "samples.jsonl"
        prior = {"status": row["status"]}
        failure_path = raw_dir / "failure.json"
        if failure_path.is_file():
            prior["failure"] = read_json(failure_path)
        if row["kind"] == "agent" and (raw_dir / "summary.json").is_file():
            summary = read_json(raw_dir / "summary.json")
            summary.update(_parse_events((raw_dir / "opencode.stdout.jsonl").read_text(encoding="utf-8")))
            (raw_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
            status = "passed" if summary.get("success") else "failed-quality"
            store.finish_run(args.run_id, datetime.now(timezone.utc).isoformat(), status, summary)
            print(json.dumps({"run_id": args.run_id, "status": status, "summary": summary}, indent=2))
            return 0
        samples = []
        if samples_path.is_file():
            samples = [json.loads(line) for line in samples_path.read_text(encoding="utf-8").splitlines() if line.strip()]
        if samples:
            summary = summarize(samples)
            summary["reconciled_from"] = prior
            status = "passed" if summary["all_quality_pass"] else "failed-quality"
            (raw_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
        else:
            summary = {"reconciled_from": prior, "error": "interrupted before any sample completed"}
            status = "interrupted"
        store.finish_run(args.run_id, datetime.now(timezone.utc).isoformat(), status, summary)
    finally:
        store.close()
    print(json.dumps({"run_id": args.run_id, "status": status, "summary": summary}, indent=2))
    return 0


def cmd_apply(args: argparse.Namespace) -> int:
    available = profiles()
    if args.profile not in available:
        raise SystemExit(f"unknown profile: {args.profile}")
    path = args.install_root / "config" / "session.json"
    session = read_json(path)
    backup = path.with_name(f"session.json.backup-{datetime.now(timezone.utc).strftime('%Y%m%d-%H%M%S')}")
    shutil.copy2(path, backup)
    session["active_profile"] = args.profile
    path.write_text(json.dumps(session, indent=2) + "\n", encoding="utf-8")
    print(f"active profile: {args.profile} (backup: {backup})")
    return 0


def installed_script(root: Path, name: str) -> Path:
    path = root / "scripts" / name
    if not path.is_file():
        raise SystemExit(f"installed script missing: {path}")
    return path


def cmd_start(args: argparse.Namespace) -> int:
    command = [powershell(), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(installed_script(args.install_root, "start-session.ps1"))]
    if args.profile:
        command += ["-Profile", args.profile]
    if args.vision:
        command.append("-Vision")
    return run(command, check=False).returncode


def cmd_stop(args: argparse.Namespace) -> int:
    return run([powershell(), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(installed_script(args.install_root, "stop-session.ps1"))], check=False).returncode


def cmd_opencode(args: argparse.Namespace) -> int:
    command = [
        powershell(), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
        str(installed_script(args.install_root, "open-local-opencode.ps1")),
        "-Project", str(args.project.resolve()), "-Profile", args.profile,
    ]
    if args.vision:
        command.append("-WithVision")
    if args.full_prompt:
        command.append("-FullPrompt")
    if args.keep_server:
        command.append("-KeepServer")
    return run(command, check=False).returncode


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="localmodel", description="Reproducible local-model lab and production control plane")
    parser.add_argument("--install-root", type=Path, default=default_install_root())
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("profiles").set_defaults(func=cmd_profiles)
    doctor = sub.add_parser("doctor")
    doctor.add_argument("--deep", action="store_true", help="hash large model artifacts")
    doctor.set_defaults(func=cmd_doctor)
    inventory = sub.add_parser("inventory")
    inventory.add_argument("--output", type=Path)
    inventory.set_defaults(func=cmd_inventory)
    benchmark = sub.add_parser("benchmark")
    benchmark.add_argument("--profile", required=True, choices=sorted(profiles()))
    benchmark.add_argument("--runs", type=int, default=10)
    benchmark.add_argument("--warmups", type=int, default=1)
    benchmark.add_argument("--workload", action="append")
    benchmark.add_argument("--keep-server", action="store_true")
    benchmark.add_argument("--notes")
    benchmark.add_argument("--no-summary", action="store_true", help="print samples and run id, but not the final JSON summary")
    benchmark.set_defaults(func=cmd_benchmark)
    context = sub.add_parser("context-stress")
    context.add_argument("--profile", required=True, choices=sorted(profiles()))
    context.add_argument("--ratio", type=float, default=0.85)
    context.add_argument("--runs", type=int, default=3)
    context.add_argument("--warmups", type=int, default=0)
    context.add_argument("--keep-server", action="store_true")
    context.add_argument("--notes")
    context.set_defaults(func=cmd_context_stress)
    agent_benchmark = sub.add_parser("agent-benchmark")
    agent_benchmark.add_argument("--profile", required=True, choices=sorted(profiles()))
    agent_benchmark.add_argument("--task", default="python-off-by-one")
    agent_benchmark.add_argument("--keep-server", action="store_true")
    agent_benchmark.add_argument("--notes")
    agent_benchmark.set_defaults(func=cmd_agent_benchmark)
    compare = sub.add_parser("compare")
    compare.add_argument("profiles", nargs="+", choices=sorted(profiles()))
    compare.set_defaults(func=cmd_compare)
    report = sub.add_parser("report")
    report.add_argument("profiles", nargs="+", choices=sorted(profiles()))
    report.add_argument("--output", type=Path)
    report.set_defaults(func=cmd_report)
    runs_parser = sub.add_parser("runs")
    runs_parser.add_argument("profiles", nargs="*")
    runs_parser.add_argument("--limit", type=int, default=20)
    runs_parser.set_defaults(func=cmd_runs)
    qualify = sub.add_parser("qualify")
    qualify.add_argument("run_id")
    qualify.add_argument("--target", choices=["candidate", "validated", "production"], default="candidate")
    qualify.set_defaults(func=cmd_qualify)
    apply = sub.add_parser("apply")
    apply.add_argument("profile", choices=sorted(profiles()))
    apply.set_defaults(func=cmd_apply)
    start = sub.add_parser("start")
    start.add_argument("--profile", choices=sorted(profiles()))
    start.add_argument("--vision", action="store_true")
    start.set_defaults(func=cmd_start)
    sub.add_parser("stop").set_defaults(func=cmd_stop)
    sub.add_parser("status").set_defaults(func=cmd_status)
    reconcile = sub.add_parser("reconcile")
    reconcile.add_argument("run_id")
    reconcile.set_defaults(func=cmd_reconcile)
    opencode = sub.add_parser("opencode")
    opencode.add_argument("--profile", default="stable-16k", choices=sorted(profiles()))
    opencode.add_argument("--project", type=Path, default=Path.cwd())
    opencode.add_argument("--vision", action="store_true")
    opencode.add_argument("--full-prompt", action="store_true", help="diagnostic only; sends OpenCode's large generic prompt")
    opencode.add_argument("--keep-server", action="store_true")
    opencode.set_defaults(func=cmd_opencode)
    return parser


def main(argv: list[str] | None = None) -> None:
    parser = build_parser()
    args = parser.parse_args(argv)
    raise SystemExit(args.func(args))


if __name__ == "__main__":
    main()
