import xml.etree.ElementTree as ET
import sys
import argparse

def main():
    parser = argparse.ArgumentParser(description="Enforce minimum coverage threshold from cobertura.xml")
    parser.add_argument("coverage_file", help="Path to cobertura.xml")
    parser.add_argument("--threshold", type=float, default=95.0, help="Minimum coverage percentage (0-100)")
    args = parser.parse_args()

    try:
        tree = ET.parse(args.coverage_file)
        root = tree.getroot()
    except Exception as e:
        print(f"Error parsing {args.coverage_file}: {e}")
        sys.exit(1)

    line_rate = root.attrib.get("line-rate")
    if line_rate is None:
        print(f"Error: Could not find 'line-rate' attribute in {args.coverage_file}")
        sys.exit(1)

    coverage_percent = float(line_rate) * 100
    
    print(f"Coverage found: {coverage_percent:.2f}%")
    print(f"Threshold required: {args.threshold:.2f}%")

    if coverage_percent < args.threshold:
        print(f"Error: Coverage {coverage_percent:.2f}% is below the required threshold of {args.threshold:.2f}%")
        sys.exit(1)
    
    print("Coverage check passed!")

if __name__ == "__main__":
    main()
