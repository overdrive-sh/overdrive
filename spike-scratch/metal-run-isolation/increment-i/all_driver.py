import subprocess
for path in ['spike-scratch/metal-run-isolation/increment-i/driver.py','spike-scratch/metal-run-isolation/increment-h/driver.py']:
    subprocess.run(['python3',path],check=True)
