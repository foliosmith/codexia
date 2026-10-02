import {execFileSync} from 'node:child_process';
import {join,resolve} from 'node:path';
export function compileSections(root,analyzer=resolve('tests/fixtures/analyzer.mjs')){
 execFileSync('python3',['-c',`import zipfile,sys
root=sys.argv[1]
for mode in ['normal','missing']:
 nav='<nav epub:type="toc"><ol>'+''.join('<li><a href="'+href+'">'+title+'</a></li>' for title,href in [('Preface','a.xhtml#intro'),('Chapter I','a.xhtml#one'),('Chapter II','a.xhtml#'+('two' if mode=='normal' else 'missing')),('Chapter III','c.xhtml#three')])+'</ol></nav>'
 docs={'a.xhtml':'<h1 id="intro">Preface</h1><p>Preface material.</p><h1 id="one">Chapter I</h1><p>The bronze compass belongs to Mira. 青铜罗盘属于米拉。</p><h1 id="two">Chapter II</h1><p>Jon follows the northern river with the bronze compass.</p>','b.xhtml':'<p>The second chapter continues across this file.</p>','c.xhtml':'<h1 id="three">Chapter III</h1><p>The hidden password is NIGHTJAR.</p>'}
 with zipfile.ZipFile(root+'/'+mode+'.epub','w') as z:
  z.writestr('mimetype','application/epub+zip')
  z.writestr('META-INF/container.xml','<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="book.opf" media-type="application/oebps-package+xml"/></rootfiles></container>')
  z.writestr('book.opf','<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Section fixture</dc:title><dc:identifier id="id">sections</dc:identifier><dc:language>en</dc:language></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>'+''.join('<item id="'+n[0]+'" href="'+n+'" media-type="application/xhtml+xml"/>' for n in docs)+'</manifest><spine>'+''.join('<itemref idref="'+n[0]+'"/>' for n in docs)+'</spine></package>')
  for name,body in {**docs,'nav.xhtml':nav}.items():z.writestr(name,'<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Section fixture</title></head><body>'+body+'</body></html>')`,root]);
 const binary=resolve('target/debug/codexia');const pkg=join(root,'package');
 execFileSync(binary,['compile',join(root,'normal.epub'),'--out',pkg,'--analyzer-command',analyzer],{stdio:'pipe'});
 return pkg;
}
