"""Regression checks for Maven license evidence selection (no network needed)."""

import io
import os
import tempfile
import unittest
import urllib.error
import zipfile
from pathlib import Path
from unittest.mock import patch

import generate_third_party_notices as collector


class MavenSourceSelectionTest(unittest.TestCase):
    COORD = "androidx.example:library-android:1.0"
    BASE = "https://dl.google.com/dl/android/maven2/androidx/example/library-android/1.0/library-android-1.0"
    APACHE_URL = "https://www.apache.org/licenses/LICENSE-2.0.txt"

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.environment = patch.dict(os.environ, {"GRADLE_USER_HOME": self.temporary.name})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.cache = Path(self.temporary.name) / "caches/modules-2/files-2.1/androidx.example/library-android/1.0"
        pom = self.cache / "pom" / "library-android-1.0.pom"
        pom.parent.mkdir(parents=True)
        pom.write_text(
            '<project xmlns="http://maven.apache.org/POM/4.0.0">'
            '<licenses><license><name>Apache-2.0</name></license></licenses></project>',
            encoding="utf-8",
        )

    @staticmethod
    def source_archive(sample=False):
        stream = io.BytesIO()
        with zipfile.ZipFile(stream, "w") as archive:
            archive.writestr("NOTICE", "Sample notice\n" if sample else "Library notice\n")
            archive.writestr("Source.kt", "// Copyright 2020 Sample Author\n" if sample else "// Copyright 2025 Library Author\n")
        return stream.getvalue()

    def cache_source(self, directory, filename, sample=False):
        path = self.cache / directory / filename
        path.parent.mkdir(parents=True)
        path.write_bytes(self.source_archive(sample))

    def component(self):
        return collector.maven_component(self.COORD, {"releaseRuntimeClasspath": [self.COORD]})

    def assert_library_evidence(self, row):
        self.assertEqual(row["copyrights"], ["Copyright 2025 Library Author"])
        notice = next(item for item in row["documents"] if item["title"] == "NOTICE")
        self.assertEqual(notice["source"], self.BASE + "-sources.jar!/NOTICE")
        self.assertEqual(collector.DOCUMENTS[notice["sha256"]], "Library notice\n")

    def test_cached_library_sources_take_precedence_over_samples(self):
        self.cache_source("a-samples", "library-1.0-samples-sources.jar", sample=True)
        self.cache_source("b-library", "library-android-1.0-sources.jar")
        with patch.object(collector, "download", return_value=b"Apache license\n") as download:
            self.assert_library_evidence(self.component())
        download.assert_called_once_with(self.APACHE_URL)

    def test_samples_only_cache_downloads_library_sources(self):
        self.cache_source("a-samples", "library-android-1.0-samples-sources.jar", sample=True)
        responses = {self.BASE + "-sources.jar": self.source_archive(), self.APACHE_URL: b"Apache license\n"}
        with patch.object(collector, "download", side_effect=responses.__getitem__) as download:
            self.assert_library_evidence(self.component())
        download.assert_any_call(self.BASE + "-sources.jar")

    def test_missing_library_sources_do_not_substitute_samples(self):
        self.cache_source("a-samples", "library-android-1.0-samples-sources.jar", sample=True)

        def download(url):
            if url == self.BASE + "-sources.jar":
                raise urllib.error.HTTPError(url, 404, "Not found", None, None)
            self.assertEqual(url, self.APACHE_URL)
            return b"Apache license\n"

        with patch.object(collector, "download", side_effect=download):
            row = self.component()
        self.assertEqual(row["copyrights"], [])
        self.assertEqual([item["title"] for item in row["documents"]], ["Apache License 2.0"])


if __name__ == "__main__":
    unittest.main()
