using System;using System.Runtime.InteropServices;using System.Text;
public static class MiaominalDetachedProcess{
[StructLayout(LayoutKind.Sequential)]struct S{public int cb;public IntPtr r,d,t;public int x,y,xs,ys,xc,yc,fa,fl;public short sw,cr;public IntPtr rr,i,o,e;}
[StructLayout(LayoutKind.Sequential)]struct P{public IntPtr p,t;public int id,tid;}
[DllImport("kernel32",SetLastError=true,CharSet=CharSet.Unicode)]static extern bool CreateProcessW(string a,StringBuilder c,IntPtr pa,IntPtr ta,bool h,uint f,IntPtr e,string d,ref S s,out P p);
[DllImport("kernel32",SetLastError=true)]static extern bool GetProcessTimes(IntPtr h,out long c,out long x,out long k,out long u);
[DllImport("kernel32")]static extern bool TerminateProcess(IntPtr h,uint c);
[DllImport("kernel32",SetLastError=true)]static extern uint ResumeThread(IntPtr h);
[DllImport("kernel32")]static extern uint WaitForSingleObject(IntPtr h,uint m);
[DllImport("kernel32")]static extern bool CloseHandle(IntPtr h);
public static long LastStartTicks;
public static int Start(string a,string g,string d){S s=new S();s.cb=Marshal.SizeOf(typeof(S));P p;uint f=0x08000204;StringBuilder c=new StringBuilder("\""+a+"\" "+g);bool ok=CreateProcessW(a,c,IntPtr.Zero,IntPtr.Zero,false,f|0x01000000,IntPtr.Zero,d,ref s,out p);if(!ok){c=new StringBuilder("\""+a+"\" "+g);ok=CreateProcessW(a,c,IntPtr.Zero,IntPtr.Zero,false,f,IntPtr.Zero,d,ref s,out p);}if(!ok)throw new Exception("CreateProcess failed: "+Marshal.GetLastWin32Error());try{long created,exited,kernel,user;if(!GetProcessTimes(p.p,out created,out exited,out kernel,out user))throw new Exception("GetProcessTimes failed: "+Marshal.GetLastWin32Error());LastStartTicks=DateTime.FromFileTimeUtc(created).Ticks;if(ResumeThread(p.t)==0xffffffff)throw new Exception("ResumeThread failed: "+Marshal.GetLastWin32Error());}catch{TerminateProcess(p.p,1);WaitForSingleObject(p.p,5000);CloseHandle(p.t);CloseHandle(p.p);throw;}CloseHandle(p.t);CloseHandle(p.p);return p.id;}}
