#!/usr/bin/env python3
"""Validate the read contract, its examples, or a saved live response.

Install requirements.txt first. This script resolves only checked-in schemas;
validation does not fetch remote resources.
"""

import argparse
import copy
import json
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parent


def load(name):
    return json.loads((ROOT / name).read_text(encoding="utf-8"))


def validator(name):
    schema = load("schema.json")
    if name not in schema["$defs"]:
        raise ValueError(f"unknown response schema: {name}")
    schema["$ref"] = f"#/$defs/{name}"
    return Draft202012Validator(schema)


def validate_response(name, value):
    validator(name).validate(value)


def validate_contract():
    schema = load("schema.json")
    Draft202012Validator.check_schema(schema)
    specification = load("openapi.json")
    assert specification["openapi"] == "3.1.0"
    assert len(specification["paths"]) == 4
    for path, item in specification["paths"].items():
        assert path.startswith("/api/hale/v1/")
        assert set(item) <= {"get", "parameters"}, "this release only advertises reads"
        for response in item["get"]["responses"].values():
            target = response["content"]["application/json"]["schema"]["$ref"]
            prefix = "./schema.json#/$defs/"
            assert target.startswith(prefix) and target[len(prefix):] in schema["$defs"]
    fixtures = load("fixtures.json")
    for example in fixtures:
        errors = list(validator(example["schema"]).iter_errors(example["value"]))
        assert bool(errors) != example["valid"], (
            f"fixture {example['name']} expected valid={example['valid']}: "
            + "; ".join(error.message for error in errors)
        )
    # Every response must reject an unexplained field, including an apparent
    # actor. Consumers must not infer authorization from uncontracted data.
    for example in fixtures:
        if example["valid"]:
            changed = copy.deepcopy(example["value"])
            changed["actor"] = "caller-selected"
            assert not validator(example["schema"]).is_valid(changed)
    print(f"contract: {len(fixtures)} fixtures passed; OpenAPI schema references resolved")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--schema", help="named response schema for --response")
    parser.add_argument("--response", type=Path, help="saved JSON response")
    args = parser.parse_args()
    if bool(args.schema) != bool(args.response):
        parser.error("--schema and --response are used together")
    validate_contract()
    if args.response:
        validate_response(args.schema, json.loads(args.response.read_text(encoding="utf-8")))
        print(f"response: {args.schema} passed")


if __name__ == "__main__":
    main()
