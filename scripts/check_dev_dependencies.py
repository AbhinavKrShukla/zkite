#!/usr/bin/env python3
"""Network integration check for the CLI using a simulated Android reload socket.

Run after `cargo build --manifest-path phase_1/cli/Cargo.toml`. Downloads pinned
pure-Python wheels from the configured pip index. Requires a POSIX host; does not
claim to test Android execution. Device-side path handling has separate Rust tests.
"""
import tempfile,pathlib,subprocess,os,socket,time,json,base64
repo=pathlib.Path(__file__).resolve().parents[1]
host_python=os.environ.get('ZKITE_PYTHON', __import__('sys').executable)
with tempfile.TemporaryDirectory() as tmp:
 root=pathlib.Path(tmp); app=root/'app'; app.mkdir(); device=root/'device'; device.mkdir()
 (app/'app.json').write_text(json.dumps({'name':'dependency-check'}))
 (app/'requirements.txt').write_text('requests==2.32.5\n')
 (app/'main.py').write_text('import requests\nassert requests.__version__ == "2.32.5"\n')
 adb=root/'adb'; adb.write_text('#!/usr/bin/env python3\nimport sys\na=sys.argv[1:]\nif "devices" in a: print("List of devices attached\\nTEST\\tdevice")\nelif "getprop" in a: print("arm64-v8a")\n');adb.chmod(0o755)
 probe=socket.socket();probe.bind(('127.0.0.1',0));port=probe.getsockname()[1];probe.close()
 env=dict(os.environ,ZKITE_PAYLOAD_ROOT=str(repo),ZKITE_PYTHON=host_python)
 with open(root/'cli.log','w') as log:
  process=subprocess.Popen([str(repo/'phase_1/cli/target/debug/zkite'),'dev','--adb',str(adb),'--serial','TEST','--port',str(port)],cwd=app,env=env,stdout=log,stderr=log)
  try:
   deadline=time.monotonic()+90
   while True:
    try: conn=socket.create_connection(('127.0.0.1',port),timeout=1);break
    except OSError:
     assert process.poll() is None, (root/'cli.log').read_text()
     assert time.monotonic()<deadline,'connection timeout'
     time.sleep(.2)
   conn.settimeout(60);conn.sendall(b'{"op":"hello","backend":"native","sync_files":true}\n');reader=conn.makefile('rb');managed=set()
   def receive():
    global managed
    count=0
    while True:
     frame=json.loads(reader.readline());op=frame['op']
     if op=='sync':
      updated=set(frame['paths'])
      for path in managed-updated: (device/path).unlink(missing_ok=True)
      managed=updated
     elif op=='file':
      path=device/frame['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(base64.b64decode(frame['content_b64']));count+=1
     elif op=='reload':return count
   print('Initial transfer:',receive(),'files',flush=True)
   python=host_python
   def run(code):subprocess.run([python,'-I','-S','-c','import sys; sys.path.insert(0,sys.argv[1]); '+code,str(device)],check=True)
   run('import main,certifi,urllib3,importlib.metadata; assert importlib.metadata.version("requests")=="2.32.5"; assert __import__("pathlib").Path(certifi.where()).is_file()')
   print('Requests, transitive imports, metadata, certificate resource: passed',flush=True)
   (app/'main.py').write_text('import requests\nvalue = 42\n')
   assert receive()==1
   print('Source-only edit transfers one file: passed',flush=True)
   (app/'requirements.txt').write_text('idna==3.10\n');(app/'main.py').write_text('import idna\nassert idna.__version__ == "3.10"\n')
   print('Dependency change transfer:',receive(),'files',flush=True)
   run('import main; import importlib.metadata; assert not any(d.metadata.get("Name") == "requests" for d in importlib.metadata.distributions(path=[sys.argv[1]]))')
   assert not (device/'requests/__init__.py').exists()
   print('Live dependency replacement/removal: passed',flush=True)
  finally:
   process.terminate();process.wait(timeout=10)
