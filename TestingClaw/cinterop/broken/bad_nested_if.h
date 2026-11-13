#ifndef BAD_IF_H
#define BAD_IF_H
#define OS_MAC 1
#if defined(OS_MAC)
  #if OS_MAC == 1
     int mac_only(void);
  #else
     int other(void);
  #endif
#endif
#endif
