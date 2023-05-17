# workaround to select Agg as backend consistenly
import os
from typing import Any, Dict, List, Union

import matplotlib as mpl  # type: ignore
import matplotlib.pyplot as plt  # type: ignore
import numpy as np
import pandas as pd
import seaborn as sns  # type: ignore

mpl.use("Agg")
mpl.rcParams["text.latex.preamble"] = r"\usepackage{amsmath}"
mpl.rcParams["pdf.fonttype"] = 42
mpl.rcParams["ps.fonttype"] = 42

sns.set(rc={"figure.figsize": (5, 5)})
sns.set_style("whitegrid")
sns.set_style("ticks", {"xtick.major.size": 8, "ytick.major.size": 8})
sns.set_context("paper", rc={"font.size": 5, "axes.titlesize": 5, "axes.labelsize": 8})

PAPER_MODE = os.environ.get("PAPER_MODE", "1") == "1"

ROW_ALIASES: Dict[str, Dict[str, str]] = dict()
COLUMN_ALIASES: Dict[str, str] = {}
FORMATTER: Dict[str, mpl.ticker.Formatter] = {}


def explode(mean: float, stddev: float) -> List[float]:
    """
    we can simplify explode_big for num_samples=2
    """
    return [mean + stddev, mean - stddev]


def explode_big(mean: float, stddev: float) -> List[float]:
    num_samples = 10
    desired_mean = mean
    desired_std_dev = stddev

    samples = np.random.normal(loc=0.0, scale=desired_std_dev, size=num_samples)

    actual_mean = np.mean(samples)
    # actual_std = np.std(samples)
    # print("Initial samples stats   : mean = {:.4f} stdv = {:.4f}".format(actual_mean, actual_std))

    zero_mean_samples = samples - (actual_mean)

    # zero_mean_mean = np.mean(zero_mean_samples)
    zero_mean_std = np.std(zero_mean_samples)
    # print("True zero samples stats : mean = {:.4f} stdv = {:.4f}".format(zero_mean_mean, zero_mean_std))

    scaled_samples = zero_mean_samples * (desired_std_dev / zero_mean_std)
    # scaled_mean = np.mean(scaled_samples)
    # scaled_std = np.std(scaled_samples)
    # print("Scaled samples stats    : mean = {:.4f} stdv = {:.4f}".format(scaled_mean, scaled_std))

    final_samples = scaled_samples + desired_mean
    # final_mean = np.mean(final_samples)
    # final_std = np.std(final_samples)
    # print("Final samples stats     : mean = {:.4f} stdv = {:.4f}".format(final_mean, final_std))
    return list(final_samples)


def systems_order(df: pd.DataFrame) -> List[str]:
    priorities: Dict[str, int] = {}
    systems = list(df.system.unique())
    return sorted(systems, key=lambda v: priorities.get(v, 100))


def catplot(**kwargs: Any) -> Any:
    # kwargs.setdefault("palette", "Greys")
    g = sns.catplot(**kwargs)
    g.despine(top=False, right=False)
    plt.autoscale()
    plt.subplots_adjust(top=0.98)
    return g


def apply_hatch(groups: int, g: Any, legend: bool) -> None:
    hatch_list = ["", "///", "---", "\\"]
    if len(g.ax.patches) == groups:
        for i, bar in enumerate(g.ax.patches):
            hatch = hatch_list[i]
            bar.set_hatch(hatch)
    else:
        for i, bar in enumerate(g.ax.patches):
            hatch = hatch_list[int(i / groups)]
            bar.set_hatch(hatch)
    if legend:
        g.ax.legend(loc="best", fontsize="small")


def rescale_barplot_width(ax: Any, factor: float = 0.6) -> None:
    for bar in ax.patches:
        x = bar.get_x()
        new_width = bar.get_width() * factor
        center = x + bar.get_width() / 2.0
        bar.set_width(new_width)
        bar.set_x(center - new_width / 2.0)


def column_alias(name: str) -> str:
    return COLUMN_ALIASES.get(name, name)


def apply_aliases(df: pd.DataFrame) -> pd.DataFrame:
    for column in df.columns:
        aliases = ROW_ALIASES.get(column, None)
        if aliases is not None:
            df[column] = df[column].replace(aliases)
    return df.rename(index=str, columns=COLUMN_ALIASES)


def magnitude_formatter(orderOfMagnitude: int) -> mpl.ticker.Formatter:
    fac = pow(10, orderOfMagnitude)
    ff = mpl.ticker.FuncFormatter(lambda val, pos: f"{val/fac:g}")
    # ff.set_offset_string(f"1e{orderOfMagnitude}")
    ff.set_offset_string("")
    return ff


def format(axis: mpl.axis.Axis, name: str) -> None:
    ff = FORMATTER[name]
    axis.set_major_formatter(ff)


def change_width(ax: Any, new_value: Union[int, float]) -> None:
    for patch in ax.patches:
        current_width = patch.get_width()
        diff = current_width - new_value
        patch.set_width(new_value)

        patch.set_x(patch.get_x() + diff * 0.5)


def set_size(w: int, h: int, ax: Any) -> None:
    """w, h: width, height in inches"""
    if not ax:
        ax = plt.gca()
    l = ax.figure.subplotpars.left
    r = ax.figure.subplotpars.right
    t = ax.figure.subplotpars.top
    b = ax.figure.subplotpars.bottom
    figw = float(w) / (r - l)
    figh = float(h) / (t - b)
    ax.figure.set_size_inches(figw, figh)


def apply_to_graphs(ax: Any, legend: bool, width: float) -> Any:
    change_width(ax, width)

    ax.set_xlabel("")
    ax.set_ylabel(ax.get_ylabel(), size=7)
    ax.set_xticklabels(ax.get_xticklabels(), size=7)
    ax.set_yticklabels(ax.get_yticklabels(), size=7)

    if legend:
        ax.legend(loc="best")
    return ax
