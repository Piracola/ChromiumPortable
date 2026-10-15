import json


def main():
    print(json.dumps({
        "version": "0.0.0.0",
        "url": "https://example.com/chrome_installer.exe",
        "file_name": "chrome_installer.exe",
        "verify_ssl": True
    }))


if __name__ == "__main__":
    main()
