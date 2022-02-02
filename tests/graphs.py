#!/usr/bin/env python3

import pandas as pd
import re
import sys
from pathlib import Path
from typing import Any
from natsort import natsort_keygen
import warnings

from plot import (
    apply_aliases,
    catplot,
    column_alias,
    explode,
    sns,
    PAPER_MODE,
    plt,
    format,
    magnitude_formatter,
)
from plot import ROW_ALIASES, COLUMN_ALIASES, FORMATTER

if PAPER_MODE:
    out_format = ".pdf"
else:
    out_format = ".png"

ROW_ALIASES.update(
    {
        "direction": dict(read_mean="read", write_mean="write"),
        "system": dict(
            direct_host1="native",
            direct_host2="native #2",
            direct_detached_qemublk=r"$\ast \dag$ qemu-blk",
            direct_ws_qemublk=r"$\dag$ wrap_syscall qemu-blk",
            direct_ws_javdev=r"$\ast$ wrap_syscall vmsh-blk",
            direct_iorefd_qemublk=r"$\dag$ ioregionfd qemu-blk",
            direct_iorefd_javdev=r"$\ast$ ioregionfd vmsh-blk",
            detached_qemublk=r"$\ddag$ qemu-blk",
            detached_qemu9p=r"$\ddag$ qemu-9p",
            attached_ws_javdev="wrap_syscall vmsh-blk",
            attached_iorefd_javdev="ioregionfd vmsh-blk",
        ),
        "iotype": dict(
            direct="Direct/Block IO",
            file="File IO",
        ),
        "benchmark_id": {
            "Compile Bench: Test: Compile [MB/s]": "Compile Bench: Compile",
            "Compile Bench: Test: Initial Create [MB/s]": "Compile Bench: Create",
            "Compile Bench: Test: Read Compiled Tree [MB/s]": "Compile Bench: Read tree",
            "Dbench: 1 Clients [MB/s]": "Dbench: 1 Client",
            "Dbench: 12 Clients [MB/s]": "Dbench: 12 Clients",
            "FS-Mark: Test: 1000 Files, 1MB Size [Files/s]": "FS-Mark: 1000 Files, 1MB",
            "FS-Mark: Test: 1000 Files, 1MB Size, No Sync/FSync [Files/s]": "FS-Mark: 1k Files, No Sync",
            "FS-Mark: Test: 4000 Files, 32 Sub Dirs, 1MB Size [Files/s]": "FS-Mark: 4k Files, 32 Dirs",
            "FS-Mark: Test: 5000 Files, 1MB Size, 4 Threads [Files/s]": "FS-Mark: 5k Files, 1MB, 4 Threads",
            "Flexible IO Tester: Type: Random Read - IO Engine: Linux AIO - Buffered: No - Direct: Yes - Block Size: 4KB - Disk Target: Default Test Directory [IOPS]": "Fio: Rand read, 4KB",
            "Flexible IO Tester: Type: Random Read - IO Engine: Linux AIO - Buffered: No - Direct: Yes - Block Size: 2MB - Disk Target: Default Test Directory [IOPS]": "Fio: Rand read, 2MB",
            "Flexible IO Tester: Type: Random Write - IO Engine: Linux AIO - Buffered: No - Direct: Yes - Block Size: 4KB - Disk Target: Default Test Directory [IOPS]": "Fio: Rand write, 4KB",
            "Flexible IO Tester: Type: Random Write - IO Engine: Linux AIO - Buffered: No - Direct: Yes - Block Size: 2MB - Disk Target: Default Test Directory [IOPS]": "Fio: Rand write, 2MB",
            "Flexible IO Tester: Type: Sequential Read - IO Engine: Linux AIO - Buffered: No - Direct: Yes - Block Size: 4KB - Disk Target: Default Test Directory [IOPS]": "Fio: Sequential read, 4KB",
            "Flexible IO Tester: Type: Sequential Read - IO Engine: Linux AIO - Buffered: No - Direct: Yes - Block Size: 2MB - Disk Target: Default Test Directory [IOPS]": "Fio: Sequential read, 2MB",
            "Flexible IO Tester: Type: Sequential Write - IO Engine: Linux AIO - Buffered: No - Direct: Yes - Block Size: 4KB - Disk Target: Default Test Directory [IOPS]": "Fio: Sequential write, 2KB",
            "Flexible IO Tester: Type: Sequential Write - IO Engine: Linux AIO - Buffered: No - Direct: Yes - Block Size: 2MB - Disk Target: Default Test Directory [IOPS]": "Fio: Sequential write, 2MB",
            "IOR: Block Size: 2MB - Disk Target: Default Test Directory [MB/s]": "IOR: 2MB",
            "IOR: Block Size: 4MB - Disk Target: Default Test Directory [MB/s]": "IOR: 4MB",
            "IOR: Block Size: 8MB - Disk Target: Default Test Directory [MB/s]": "IOR: 8MB",
            "IOR: Block Size: 16MB - Disk Target: Default Test Directory [MB/s]": "IOR: 16MB",
            "IOR: Block Size: 32MB - Disk Target: Default Test Directory [MB/s]": "IOR: 32MB",
            "IOR: Block Size: 64MB - Disk Target: Default Test Directory [MB/s]": "IOR: 64MB",
            "IOR: Block Size: 256MB - Disk Target: Default Test Directory [MB/s]": "IOR: 256MB",
            "IOR: Block Size: 512MB - Disk Target: Default Test Directory [MB/s]": "IOR: 512MB",
            "IOR: Block Size: 1024MB - Disk Target: Default Test Directory [MB/s]": "IOR: 1025MB",
            "PostMark: Disk Transaction Performance [TPS]": "PostMark: Disk transactions",
            "SQLite: Threads / Copies: 1 [Seconds]": "Sqlite: 1 Threads",
            "SQLite: Threads / Copies: 8 [Seconds]": "Sqlite: 8 Threads",
            "SQLite: Threads / Copies: 32 [Seconds]": "Sqlite: 32 Threads",
            "SQLite: Threads / Copies: 64 [Seconds]": "Sqlite: 64 Threads",
            "SQLite: Threads / Copies: 128 [Seconds]": "Sqlite: 128 Threads",
            "AIO-Stress: Random Write": "AIO-Stress: Random Write",
            "SQLite: Timed SQLite Insertions": "SQlite",
            "FS-Mark: 1000 Files, 1MB Size": "FS-Mark",
        },
    }
)

ROW_ALIASES["system"]["vmsh-console"] = "vmsh-console"

COLUMN_ALIASES.update(
    {
        "container_size": "image size [MB]",
        "iops": "IOPS [k]",
        "io_throughput": "Throughput [GB/s]",
        "direction": "Direction",
        "seconds": "latency [ms]",
    }
)
FORMATTER.update(
    {
        "iops": magnitude_formatter(3),
        "io_throughput": magnitude_formatter(6),
        "seconds": magnitude_formatter(-3),
    }
)


def image_sizes(df: pd.DataFrame) -> Any:
    print("Size reduction")
    reduction = 100 * (1 - (df.new_size / df.old_size))
    print(reduction.describe())

    df2 = df.assign(reduction=reduction)
    not_effective = df2[df2.reduction <= 10]
    print(
        f"Size reduction smaller than 10%\n{not_effective}\n{not_effective.describe()}"
    )
    df_before = df.assign(when="before", container_size=lambda x: x.old_size / 10e6)
    df_after = df.assign(when="after", container_size=lambda x: x.new_size / 10e6)
    merged = pd.concat([df_before, df_after])

    sns.set(font_scale=1.3)
    sns.set_style("whitegrid")
    g = sns.boxplot(
        y=column_alias("container_size"),
        x=column_alias("when"),
        data=apply_aliases(merged),
        palette=None,
    )
    g.axes.set_xlabel("")
    g.set(ylim=(-1, 100))
    g.get_figure().set_figheight(3.3)
    plt.gcf().tight_layout()
    FONT_SIZE = 12
    g.annotate(
        "Lower is better",
        xycoords="axes fraction",
        xy=(0, 0),
        xytext=(1.02, 0.17),
        fontsize=FONT_SIZE,
        color="navy",
        weight="bold",
        rotation="vertical",
    )
    g.annotate(
        "",
        xycoords="axes fraction",
        xy=(1.04, 0.05),
        xytext=(1.04, 0.15),
        fontsize=FONT_SIZE,
        arrowprops=dict(arrowstyle="-|>", color="navy"),
    )
    # apply_to_graphs(g, legend=False, width=0.28)
    return plt.gcf()


def gobshit_to_stddev(df: pd.DataFrame) -> pd.DataFrame:
    df.insert(df.shape[1], "stddev", [0 for _ in range(df.shape[0])])

    def f(row: Any) -> Any:
        if row.direction == "read_mean":
            row.stddev = row.read_stddev
        elif row.direction == "write_mean":
            row.stddev = row.write_stddev
        return row

    df = df.apply(f, axis=1)
    del df["read_stddev"]
    del df["write_stddev"]
    return df


def stddev_to_series(df: pd.DataFrame, mean: str, stddev: str) -> pd.DataFrame:
    ret = pd.DataFrame()
    for _index, row in df.iterrows():
        samples = explode(row[mean], row[stddev])
        for sample in samples:
            row[mean] = sample
            ret = ret.append(row)
    del ret[stddev]
    return ret


def system_to_iotype(df: pd.DataFrame, value: str) -> pd.DataFrame:
    def f(row: Any) -> Any:
        if "direct" in row.system:
            return "direct"
        else:
            return "file"

    iotype = df.apply(f, axis=1)
    return df.assign(iotype=iotype)


def fio(df: pd.DataFrame, what: str, value_name: str) -> Any:
    df = df[df["benchmark"] == what][df["system"] != "direct_host2"]
    df = df.melt(
        id_vars=["system", "benchmark", "Unnamed: 0", "write_stddev", "read_stddev"],
        var_name="direction",
        value_name=value_name,
    )
    df = gobshit_to_stddev(df)
    df = system_to_iotype(df, value_name)
    df = stddev_to_series(df, value_name, "stddev")

    directs = sum([int(t == "direct") for t in df["iotype"]])
    files = sum([int(t == "file") for t in df["iotype"]])
    g = catplot(
        data=apply_aliases(df),
        y=column_alias("system"),
        # order=systems_order(df),
        x=column_alias(value_name),
        hue=column_alias("direction"),
        kind="bar",
        ci="sd",  # show standard deviation! otherwise with_stddev_to_long_form does not work.
        height=2.3,
        aspect=2,
        palette=None,
        legend=False,
        row="iotype",
        sharex=True,
        sharey=False,
        facet_kws=dict({"gridspec_kws": {"height_ratios": [directs, files]}}),
    )
    # apply_to_graphs(g.ax, False, 0.285)
    g.axes[1][0].legend(
        loc="lower right", frameon=True, title=column_alias("direction")
    )
    # g.axes[0][0].set_title("Direct/Block IO", size=10)
    # g.axes[1][0].set_title("File IO", size=10)
    g.axes[0][0].set_ylabel("")
    g.axes[1][0].set_ylabel("")
    g.axes[0][0].grid()
    g.axes[1][0].grid()
    # g.axes[0][0].set_xlim([0, 200000])
    # g.axes[1][0].set_xlim([0, 30000])
    # if "iops" in value_name:
    # g.axes[0][0].set_xscale("log")
    # g.axes[1][0].set_xscale("log")
    FONT_SIZE = 7.5
    g.axes[1][0].annotate(
        "Higher is better",
        xycoords="axes fraction",
        xy=(0, 0),
        xytext=(-0.47, -0.25),
        fontsize=FONT_SIZE,
        color="navy",
        weight="bold",
    )
    g.axes[1][0].annotate(
        "",
        xycoords="axes fraction",
        xy=(-0.04, -0.23),
        xytext=(-0.13, -0.23),
        fontsize=FONT_SIZE,
        arrowprops=dict(arrowstyle="-|>", color="navy"),
    )
    format(g.axes[0][0].xaxis, value_name)
    format(g.axes[1][0].xaxis, value_name)
    return g


def console(df: pd.DataFrame) -> Any:
    df = df.melt(id_vars=["Unnamed: 0"], var_name="system", value_name="seconds")
    # df = df.append(dict(system=r"human", seconds=0.013), ignore_index=True)
    g = catplot(
        data=apply_aliases(df),
        y=column_alias("system"),
        # order=systems_order(df),
        x=column_alias("seconds"),
        kind="bar",
        ci="sd",  # show standard deviation! otherwise with_stddev_to_long_form does not work.
        height=1.1,
        aspect=4,
        palette=None,
    )
    # apply_to_graphs(g.ax, False, 0.285)
    g.ax.set_ylabel("")
    FONT_SIZE = 8
    g.ax.annotate(
        "Lower is better",
        xycoords="axes fraction",
        xy=(0, 0),
        xytext=(0.07, -0.62),
        fontsize=FONT_SIZE,
        color="navy",
        weight="bold",
    )
    g.ax.annotate(
        "",
        xycoords="axes fraction",
        xy=(0.0, -0.56),
        xytext=(0.07, -0.56),
        fontsize=FONT_SIZE,
        arrowprops=dict(arrowstyle="-|>", color="navy"),
    )
    g.despine()
    format(g.ax.xaxis, "seconds")
    return g


def compute_ratio(x: pd.Dataframe) -> pd.Series:
    title = x.benchmark_title.iloc[0]
    scale = x.scale.iloc[0]
    native = x.value.iloc[0]
    if len(x.value) == 2:
        vmsh = x.value.iloc[1]
    else:
        print(f"WARNING: found only values for {title} for {x.identifier.iloc[0]}")
        # FIXME
        import math

        vmsh = math.nan
    if x.proportion.iloc[0] == "LIB":
        diff = vmsh / native
        proportion = "lower is better"
    else:
        diff = native / vmsh
        proportion = "higher is better"

    result = dict(
        title=x.title.iloc[0],
        benchmark_title=title,
        benchmark_group=x.benchmark_name,
        diff=diff,
        native=native,
        vmsh=vmsh,
        scale=scale,
        proportion=proportion,
    )
    return pd.Series(result, name="metrics")


CONVERSION_MAPPING = {
    "MB": 10e6,
    "KB": 10e3,
}

ALL_UNITS = "|".join(CONVERSION_MAPPING.keys())
UNIT_FINDER = re.compile(r"(\d+)\s*({})".format(ALL_UNITS), re.IGNORECASE)


def unit_replacer(matchobj: re.Match) -> str:
    """Given a regex match object, return a replacement string where units are modified"""
    number = matchobj.group(1)
    unit = matchobj.group(2)
    new_number = int(number) * CONVERSION_MAPPING[unit]
    return f"{new_number} B"


def sort_row(val: pd.Series) -> int:
    return natsort_keygen()(val.apply(lambda v: UNIT_FINDER.sub(unit_replacer, v)))


def bar_colors(graph: Any, df: pd.Series, num_colors: int) -> None:
    colors = sns.color_palette(n_colors=num_colors)
    groups = 0
    last_group = df[0].iloc[0]
    for i, patch in enumerate(graph.axes[0][0].patches):
        if last_group != df[i].iloc[0]:
            last_group = df[i].iloc[0]
            groups += 1
        patch.set_facecolor(colors[groups])


def phoronix(df: pd.DataFrame) -> Any:
    df = df[df["identifier"].isin(["vmsh-blk", "qemu-blk"])]
    groups = len(df.benchmark_name.unique())
    # same benchmark with different units
    df = df[~((df.benchmark_name.str.startswith("pts/fio")) & (df.scale == "MB/s"))]
    df = df.sort_values(by=["benchmark_id", "identifier"], key=sort_row)
    df = df.groupby("benchmark_id").apply(compute_ratio).reset_index()
    df = df.sort_values(by=["benchmark_id"], key=sort_row)
    g = catplot(
        data=apply_aliases(df),
        y=column_alias("benchmark_id"),
        x=column_alias("diff"),
        kind="bar",
        palette=None,
    )
    bar_colors(g, df.benchmark_group, groups)
    g.ax.set_xlabel("")
    g.ax.set_ylabel("")
    FONT_SIZE = 9
    g.ax.annotate(
        "Lower is better",
        xycoords="axes fraction",
        xy=(0, 0),
        xytext=(0.1, -0.08),
        fontsize=FONT_SIZE,
        color="navy",
        weight="bold",
    )
    g.ax.annotate(
        "",
        xycoords="axes fraction",
        xy=(0.0, -0.07),
        xytext=(0.1, -0.07),
        fontsize=FONT_SIZE,
        arrowprops=dict(arrowstyle="-|>", color="navy"),
    )
    g.ax.axvline(x=1, color="gray", linestyle=":")
    g.ax.annotate(
        "baseline",
        xy=(1.1, -0.2),
        fontsize=FONT_SIZE,
    )
    return g


def fio_overhead(df: pd.DataFrame, what: str, value_name: str) -> Any:
    df = df[df["benchmark"] == what]
    df = df.melt(
        id_vars=["system", "benchmark", "Unnamed: 0", "write_stddev", "read_stddev"],
        var_name="direction",
        value_name=value_name,
    )
    df = gobshit_to_stddev(df)
    df = system_to_iotype(df, value_name)
    warnings.simplefilter("ignore")
    fr = df[df.iotype == "file"][df.direction == "read_mean"]
    fw = df[df.iotype == "file"][df.direction == "write_mean"]
    dr = df[df.iotype == "direct"][df.direction == "read_mean"]
    dw = df[df.iotype == "direct"][df.direction == "write_mean"]
    warnings.simplefilter("default")

    def foo(g: pd.Dataframe, system: str) -> pd.Dataframe:
        df = pd.DataFrame()
        mean = float(g[g.system == system][value_name])
        stddev = float(g[g.system == system]["stddev"])

        def f(row: pd.Dataframe) -> pd.Dataframe:
            # row["stddev"] /= row[value_name]
            # if str(row.system) != system:
            #    from math import sqrt
            #    row["stddev"] = sqrt(pow(row["stddev"], 2) * pow(stddev/mean, 2))
            #    row["stddev"] *= stddev/mean
            # row[value_name] /= mean
            #
            # i dont think there actually exsists a stddev of this:
            # https://en.wikipedia.org/wiki/Ratio_distribution#Uncorrelated_central_normal_ratio
            row["stddev"] /= row[value_name]
            if str(row.system) != system:
                row["stddev"] += stddev / mean
            row[value_name] = mean / row[value_name]
            row["stddev"] *= row[value_name]
            #
            # first try:
            # row[value_name] /= mean
            # row["stddev"] += stddev
            # row["stddev"] /= mean
            return row

        g = g.apply(f, axis=1)
        df = df.append(g)
        return df

    fr = foo(fr, "detached_qemublk")
    fw = foo(fw, "detached_qemublk")
    dr = foo(dr, "direct_detached_qemublk")
    dw = foo(dw, "direct_detached_qemublk")
    df = pd.concat([dr, fr, dw, fw], sort=True)  # TODO fix sorting
    df = stddev_to_series(df, value_name, "stddev")
    directs = sum([int(t == "direct") for t in df["iotype"]])
    files = sum([int(t == "file") for t in df["iotype"]])
    g = catplot(
        data=apply_aliases(df),
        y=column_alias("system"),
        # order=systems_order(df),
        x=column_alias(value_name),
        hue=column_alias("direction"),
        kind="bar",
        ci="sd",  # show standard deviation! otherwise with_stddev_to_long_form does not work.
        height=2.3,
        aspect=2,
        # color=color,
        palette=None,
        legend=False,
        row="iotype",
        sharex=False,
        sharey=False,
        facet_kws=dict({"gridspec_kws": {"height_ratios": [directs, files]}}),
    )
    g.axes[0][0].legend(
        loc="upper right", frameon=True, title=column_alias("direction")
    )
    g.axes[1][0].set_xlabel("Overhead: " + g.axes[1][0].get_xlabel())
    g.axes[0][0].set_ylabel("")
    g.axes[1][0].set_ylabel("")
    g.axes[0][0].grid()
    g.axes[1][0].grid()
    return g


def main() -> None:
    if len(sys.argv) < 2:
        print(f"USAGE: {sys.argv[0]} graph.tsv...")
    graphs = []
    for arg in sys.argv[1:]:
        tsv_path = Path(arg)
        df = pd.read_csv(tsv_path, sep="\t")
        assert isinstance(df, pd.DataFrame)
        name = tsv_path.stem

        if name == "docker-images":
            graphs.append(("docker-images", image_sizes(df)))
        elif name.startswith("fio"):
            graphs.append(
                (
                    "fio-best-case-bw-seperate",
                    fio(df, "best-case-bw-seperate", "io_throughput"),
                )
            )
            graphs.append(
                (
                    "fio-worst-case-iops-seperate",
                    fio(df, "worst-case-iops-seperate", "iops"),
                )
            )
            graphs.append(
                (
                    "fio-best-case-bw-seperate_overhead",
                    fio_overhead(df, "best-case-bw-seperate", "io_throughput"),
                )
            )
            graphs.append(
                (
                    "fio-worst-case-iops-seperate_overhead",
                    fio_overhead(df, "worst-case-iops-seperate", "iops"),
                )
            )
        elif name.startswith("console"):
            graphs.append(("console", console(df)))
        elif name.startswith("phoronix"):
            graphs.append(("phoronix", phoronix(df)))
        else:
            print(f"unhandled graph name: {tsv_path}", file=sys.stderr)
            sys.exit(1)

    for prefix, graph in graphs:
        fname = f"{prefix}{out_format}"
        print(f"write {fname}")
        graph.savefig(fname)


if __name__ == "__main__":
    main()
