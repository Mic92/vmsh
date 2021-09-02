from lxml import etree
from collections import defaultdict
from typing import Union
from pathlib import Path
import pandas as pd


def parse_xml(path: Union[str, Path]) -> pd.DataFrame:
    tree = etree.parse(str(path))
    results = defaultdict(list)
    for result in tree.xpath("./Result"):
        for entry in result.xpath("./Data/Entry"):
            results["identifier"].append(entry.xpath("./Identifier")[0].text)
            results["value"].append(float(entry.xpath("./Value")[0].text))
            results["raw_string"].append(entry.xpath("./RawString")[0].text)
            json = entry.xpath("./JSON")
            if len(json) == 0:
                results["json"].append("")
            else:
                results["json"].append(json[0].text)

            results["benchmark_name"].append(result.xpath("./Identifier")[0].text)
            title = result.xpath("./Title")[0].text
            scale = result.xpath("./Scale")[0].text
            description = result.xpath("./Description")[0].text

            results["title"].append(title)
            results["app_version"].append(result.xpath("./AppVersion")[0].text)
            results["description"].append(description)
            results["scale"].append(scale)

            results["proportion"].append(result.xpath("./Proportion")[0].text)

            # FIXME
            if results["benchmark_name"][-1] is None:
                assert results["title"][-1] == "Flexible IO Tester"
                results["benchmark_name"][-1] = "pts/fio-1.9.0"

            results["benchmark_title"].append("%s: %s" % (title, description))
            results["benchmark_id"].append("%s: %s [%s]" % (title, description, scale))

    return pd.DataFrame(results)
