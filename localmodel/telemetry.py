from __future__ import annotations

import ctypes
import statistics
import subprocess
import threading
from ctypes import wintypes


class GpuTelemetry:
    FIELDS = (
        "utilization.gpu,utilization.memory,memory.used,clocks.sm,clocks.mem,"
        "power.draw,temperature.gpu"
    )

    def __init__(self, interval_ms: int = 250):
        self.interval_ms = interval_ms
        self.samples: list[list[float]] = []
        self.process: subprocess.Popen[str] | None = None
        self.thread: threading.Thread | None = None

    def start(self) -> None:
        command = [
            "nvidia-smi", f"--query-gpu={self.FIELDS}", "--format=csv,noheader,nounits",
            f"--loop-ms={self.interval_ms}",
        ]
        self.process = subprocess.Popen(
            command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, encoding="utf-8", errors="replace", bufsize=1,
        )
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()

    def _read(self) -> None:
        assert self.process and self.process.stdout
        for line in self.process.stdout:
            try:
                values = [float(item.strip()) for item in line.split(",")]
                if len(values) == 7:
                    self.samples.append(values)
            except ValueError:
                continue

    def stop(self) -> dict[str, float | int | None]:
        if self.process:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
        if self.thread:
            self.thread.join(timeout=2)
        if not self.samples:
            return {"sample_count": 0, "vram_peak_mib": None, "gpu_util_mean": None, "gpu_memory_util_mean": None, "gpu_clock_mean_mhz": None, "memory_clock_mean_mhz": None, "gpu_power_mean_w": None, "gpu_temp_max_c": None}
        columns = list(zip(*self.samples))
        return {
            "sample_count": len(self.samples),
            "gpu_util_mean": statistics.fmean(columns[0]),
            "gpu_memory_util_mean": statistics.fmean(columns[1]),
            "vram_peak_mib": max(columns[2]),
            "gpu_clock_mean_mhz": statistics.fmean(columns[3]),
            "memory_clock_mean_mhz": statistics.fmean(columns[4]),
            "gpu_power_mean_w": statistics.fmean(columns[5]),
            "gpu_temp_max_c": max(columns[6]),
        }


class PROCESS_MEMORY_COUNTERS_EX(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t),
        ("PrivateUsage", ctypes.c_size_t),
    ]


def process_memory(pid: int | None) -> dict[str, float | int | None]:
    if not pid or not hasattr(ctypes, "windll"):
        return {"working_set_mib": None, "private_mib": None, "page_faults": None}
    handle = ctypes.windll.kernel32.OpenProcess(0x0400 | 0x0010, False, pid)
    if not handle:
        return {"working_set_mib": None, "private_mib": None, "page_faults": None}
    counters = PROCESS_MEMORY_COUNTERS_EX()
    counters.cb = ctypes.sizeof(counters)
    try:
        ok = ctypes.windll.psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb)
        if not ok:
            return {"working_set_mib": None, "private_mib": None, "page_faults": None}
        return {
            "working_set_mib": counters.WorkingSetSize / (1024 * 1024),
            "private_mib": counters.PrivateUsage / (1024 * 1024),
            "page_faults": int(counters.PageFaultCount),
        }
    finally:
        ctypes.windll.kernel32.CloseHandle(handle)
