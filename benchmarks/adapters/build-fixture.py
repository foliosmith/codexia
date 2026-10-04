import json
import sys
import zipfile
from html import escape

with open(sys.argv[1], encoding="utf-8") as source:
    book = json.load(source)

def document(body):
    return ('<html xmlns="http://www.w3.org/1999/xhtml" '
            'xmlns:epub="http://www.idpf.org/2007/ops"><head><title>'
            + escape(book["title"]) + '</title></head><body>' + body + '</body></html>')

entries = {
    "mimetype": "application/epub+zip",
    "META-INF/container.xml": '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="book.opf" media-type="application/oebps-package+xml"/></rootfiles></container>',
    "book.opf": '<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>' + escape(book["title"]) + '</dc:title><dc:identifier id="id">' + escape(book["id"]) + '</dc:identifier><dc:language>en</dc:language></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>' + ''.join('<item id="c%d" href="%s" media-type="application/xhtml+xml"/>' % (i, c["href"]) for i, c in enumerate(book["chapters"])) + '</manifest><spine>' + ''.join('<itemref idref="c%d"/>' % i for i in range(len(book["chapters"]))) + '</spine></package>',
    "nav.xhtml": document('<nav epub:type="toc"><ol>' + ''.join('<li><a href="' + c["href"] + '">' + escape(c["title"]) + '</a></li>' for c in book["chapters"]) + '</ol></nav>'),
}
for chapter in book["chapters"]:
    entries[chapter["href"]] = document('<h1>' + escape(chapter["title"]) + '</h1>' + ''.join('<p id="' + p["id"] + '">' + escape(p["text"]) + '</p>' for p in chapter["paragraphs"]))
with zipfile.ZipFile(sys.argv[2], "x") as archive:
    for name, content in entries.items():
        archive.writestr(zipfile.ZipInfo(name, (2000, 1, 1, 0, 0, 0)), content.encode("utf-8"))
