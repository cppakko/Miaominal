"""Check English resource coverage and Android format argument compatibility."""
from collections import Counter
from pathlib import Path
import re
import xml.etree.ElementTree as ET

RES = Path(__file__).resolve().parents[1] / "app/src/main/res"
FORMAT = re.compile(r"%(?:(\d+)\$)?[-#+ 0,(]*\d*(?:\.\d+)?([a-zA-Z%])")


def strings(directory):
    result = {}
    for path in sorted(directory.glob("*.xml")):
        for item in ET.parse(path).getroot():
            if item.tag != "string" or item.get("translatable") == "false":
                continue
            name = item.attrib["name"]
            assert name not in result, f"Duplicate resource: {directory.name}/{name}"
            result[name] = "".join(item.itertext())
    return result


def arguments(text):
    return Counter((index, kind) for index, kind in FORMAT.findall(text) if kind != "%")


def main():
    source = strings(RES / "values")
    english = strings(RES / "values-en")
    assert source.keys() == english.keys(), (
        f"Missing English: {source.keys() - english.keys()}; "
        f"Unknown English: {english.keys() - source.keys()}"
    )
    for key, text in source.items():
        assert arguments(text) == arguments(english[key]), f"Format argument mismatch: {key}"
        assert key == "language_chinese" or not re.search(r"[\u3400-\u9fff]", english[key]), (
            f"Untranslated English resource: {key}"
        )
    print(f"Verified {len(source)} resource pairs: complete English coverage and matching format arguments.")


if __name__ == "__main__":
    main()
