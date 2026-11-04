// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <time.h>

#define MAX_INPUT 256
#define MAX_COMMAND 64
#define MAX_ARGS 10
#define MAX_FILES 20
#define MAX_USERS 10
#define MAX_VARIABLES 20

typedef struct {
    char name[32];
    char password[32];
    int permission_level;
    int logged_in;
} user_t;

typedef struct {
    char filename[64];
    char content[512];
    char owner[32];
    int permissions;
} file_t;

typedef struct {
    char name[32];
    char value[128];
} variable_t;

// Global state
static user_t users[MAX_USERS];
static int user_count = 0;
static user_t *current_user = NULL;

static file_t files[MAX_FILES];
static int file_count = 0;

static variable_t variables[MAX_VARIABLES];
static int variable_count = 0;

static int debug_mode = 0;
static int verbose_mode = 0;

// Parse command and arguments
void parse_command(const char *input, char *cmd, char args[][MAX_COMMAND], int *arg_count) {
    char temp[MAX_INPUT];
    strncpy(temp, input, MAX_INPUT - 1);
    temp[MAX_INPUT - 1] = '\0';
    
    *arg_count = 0;
    char *token = strtok(temp, " \t");
    
    if (token) {
        strncpy(cmd, token, MAX_COMMAND - 1);
        cmd[MAX_COMMAND - 1] = '\0';
        
        while ((token = strtok(NULL, " \t")) && *arg_count < MAX_ARGS) {
            strncpy(args[*arg_count], token, MAX_COMMAND - 1);
            args[*arg_count][MAX_COMMAND - 1] = '\0';
            (*arg_count)++;
        }
    }
}

// User management commands
void cmd_adduser(char args[][MAX_COMMAND], int arg_count) {
    if (arg_count < 2) {
        printf("Usage: adduser <username> <password> [permission_level]\n");
        return;
    }
    
    if (user_count >= MAX_USERS) {
        printf("Error: Maximum users reached\n");
        return;
    }
    
    // Check if user already exists using strcmp
    for (int i = 0; i < user_count; i++) {
        if (strcmp(users[i].name, args[0]) == 0) {
            printf("Error: User '%s' already exists\n", args[0]);
            return;
        }
    }
    
    strcpy(users[user_count].name, args[0]);
    strcpy(users[user_count].password, args[1]);
    users[user_count].permission_level = (arg_count >= 3) ? atoi(args[2]) : 1;
    users[user_count].logged_in = 0;
    user_count++;
    
    printf("User '%s' added with permission level %d\n", 
           args[0], users[user_count - 1].permission_level);
}

void cmd_login(char args[][MAX_COMMAND], int arg_count) {
    if (arg_count < 2) {
        printf("Usage: login <username> <password>\n");
        return;
    }
    
    if (current_user && current_user->logged_in) {
        printf("Error: User '%s' already logged in. Use 'logout' first.\n", 
               current_user->name);
        return;
    }
    
    // Find user and verify password using strcmp
    for (int i = 0; i < user_count; i++) {
        if (strcmp(users[i].name, args[0]) == 0) {
            if (strcmp(users[i].password, args[1]) == 0) {
                users[i].logged_in = 1;
                current_user = &users[i];
                printf("Login successful. Welcome, %s!\n", current_user->name);
                return;
            } else {
                printf("Error: Incorrect password\n");
                return;
            }
        }
    }
    
    printf("Error: User not found\n");
}

void cmd_logout(void) {
    if (!current_user || !current_user->logged_in) {
        printf("Error: No user logged in\n");
        return;
    }
    
    printf("Goodbye, %s!\n", current_user->name);
    current_user->logged_in = 0;
    current_user = NULL;
}

void cmd_whoami(void) {
    if (!current_user || !current_user->logged_in) {
        printf("Not logged in\n");
        return;
    }
    
    printf("Current user: %s\n", current_user->name);
    printf("Permission level: %d\n", current_user->permission_level);
}

void cmd_listusers(void) {
    if (user_count == 0) {
        printf("No users registered\n");
        return;
    }
    
    printf("Registered users:\n");
    for (int i = 0; i < user_count; i++) {
        printf("  %s (level %d) %s\n", 
               users[i].name, 
               users[i].permission_level,
               users[i].logged_in ? "[logged in]" : "");
    }
}

// File management commands
void cmd_createfile(char args[][MAX_COMMAND], int arg_count) {
    if (!current_user || !current_user->logged_in) {
        printf("Error: Must be logged in\n");
        return;
    }
    
    if (arg_count < 1) {
        printf("Usage: createfile <filename> [content]\n");
        return;
    }
    
    if (file_count >= MAX_FILES) {
        printf("Error: Maximum files reached\n");
        return;
    }
    
    // Check if file exists using strcmp
    for (int i = 0; i < file_count; i++) {
        if (strcmp(files[i].filename, args[0]) == 0) {
            printf("Error: File '%s' already exists\n", args[0]);
            return;
        }
    }
    
    strcpy(files[file_count].filename, args[0]);
    strcpy(files[file_count].owner, current_user->name);
    files[file_count].permissions = 755;
    
    if (arg_count >= 2) {
        strcpy(files[file_count].content, args[1]);
    } else {
        files[file_count].content[0] = '\0';
    }
    
    file_count++;
    printf("File '%s' created\n", args[0]);
}

void cmd_readfile(char args[][MAX_COMMAND], int arg_count) {
    if (arg_count < 1) {
        printf("Usage: readfile <filename>\n");
        return;
    }
    
    // Find file using strcmp
    for (int i = 0; i < file_count; i++) {
        if (strcmp(files[i].filename, args[0]) == 0) {
            printf("=== %s ===\n", files[i].filename);
            printf("Owner: %s\n", files[i].owner);
            printf("Permissions: %d\n", files[i].permissions);
            printf("Content: %s\n", files[i].content);
            return;
        }
    }
    
    printf("Error: File '%s' not found\n", args[0]);
}

void cmd_writefile(char args[][MAX_COMMAND], int arg_count) {
    if (!current_user || !current_user->logged_in) {
        printf("Error: Must be logged in\n");
        return;
    }
    
    if (arg_count < 2) {
        printf("Usage: writefile <filename> <content>\n");
        return;
    }
    
    // Find file and check ownership
    for (int i = 0; i < file_count; i++) {
        if (strcmp(files[i].filename, args[0]) == 0) {
            // Check if current user owns the file
            if (strcmp(files[i].owner, current_user->name) == 0 || 
                current_user->permission_level >= 5) {
                strcpy(files[i].content, args[1]);
                printf("File '%s' updated\n", args[0]);
                return;
            } else {
                printf("Error: Permission denied\n");
                return;
            }
        }
    }
    
    printf("Error: File '%s' not found\n", args[0]);
}

void cmd_deletefile(char args[][MAX_COMMAND], int arg_count) {
    if (!current_user || !current_user->logged_in) {
        printf("Error: Must be logged in\n");
        return;
    }
    
    if (arg_count < 1) {
        printf("Usage: deletefile <filename>\n");
        return;
    }
    
    // Find and delete file
    for (int i = 0; i < file_count; i++) {
        if (strcmp(files[i].filename, args[0]) == 0) {
            if (strcmp(files[i].owner, current_user->name) == 0 || 
                current_user->permission_level >= 9) {
                // Shift remaining files
                for (int j = i; j < file_count - 1; j++) {
                    files[j] = files[j + 1];
                }
                file_count--;
                printf("File '%s' deleted\n", args[0]);
                return;
            } else {
                printf("Error: Permission denied\n");
                return;
            }
        }
    }
    
    printf("Error: File '%s' not found\n", args[0]);
}

void cmd_listfiles(void) {
    if (file_count == 0) {
        printf("No files\n");
        return;
    }
    
    printf("Files:\n");
    for (int i = 0; i < file_count; i++) {
        printf("  %s (owner: %s, perm: %d)\n", 
               files[i].filename, files[i].owner, files[i].permissions);
    }
}

// Variable commands
void cmd_set(char args[][MAX_COMMAND], int arg_count) {
    if (arg_count < 2) {
        printf("Usage: set <name> <value>\n");
        return;
    }
    
    // Check if variable exists
    for (int i = 0; i < variable_count; i++) {
        if (strcmp(variables[i].name, args[0]) == 0) {
            strcpy(variables[i].value, args[1]);
            printf("Variable '%s' updated\n", args[0]);
            return;
        }
    }
    
    // Create new variable
    if (variable_count >= MAX_VARIABLES) {
        printf("Error: Maximum variables reached\n");
        return;
    }
    
    strcpy(variables[variable_count].name, args[0]);
    strcpy(variables[variable_count].value, args[1]);
    variable_count++;
    printf("Variable '%s' set\n", args[0]);
}

void cmd_get(char args[][MAX_COMMAND], int arg_count) {
    if (arg_count < 1) {
        printf("Usage: get <name>\n");
        return;
    }
    
    for (int i = 0; i < variable_count; i++) {
        if (strcmp(variables[i].name, args[0]) == 0) {
            printf("%s = %s\n", variables[i].name, variables[i].value);
            return;
        }
    }
    
    printf("Error: Variable '%s' not found\n", args[0]);
}

void cmd_unset(char args[][MAX_COMMAND], int arg_count) {
    if (arg_count < 1) {
        printf("Usage: unset <name>\n");
        return;
    }
    
    for (int i = 0; i < variable_count; i++) {
        if (strcmp(variables[i].name, args[0]) == 0) {
            for (int j = i; j < variable_count - 1; j++) {
                variables[j] = variables[j + 1];
            }
            variable_count--;
            printf("Variable '%s' unset\n", args[0]);
            return;
        }
    }
    
    printf("Error: Variable '%s' not found\n", args[0]);
}

void cmd_listvars(void) {
    if (variable_count == 0) {
        printf("No variables set\n");
        return;
    }
    
    printf("Variables:\n");
    for (int i = 0; i < variable_count; i++) {
        printf("  %s = %s\n", variables[i].name, variables[i].value);
    }
}

// String comparison commands
void cmd_compare(char args[][MAX_COMMAND], int arg_count) {
    if (arg_count < 2) {
        printf("Usage: compare <string1> <string2>\n");
        return;
    }
    
    int result = strcmp(args[0], args[1]);
    
    printf("strcmp('%s', '%s') = %d\n", args[0], args[1], result);
    
    if (result == 0) {
        printf("Strings are equal\n");
    } else if (result < 0) {
        printf("'%s' < '%s'\n", args[0], args[1]);
    } else {
        printf("'%s' > '%s'\n", args[0], args[1]);
    }
}

void cmd_compareN(char args[][MAX_COMMAND], int arg_count) {
    if (arg_count < 3) {
        printf("Usage: compareN <string1> <string2> <n>\n");
        return;
    }
    
    int n = atoi(args[2]);
    int result = strncmp(args[0], args[1], n);
    
    printf("strncmp('%s', '%s', %d) = %d\n", args[0], args[1], n, result);
    
    if (result == 0) {
        printf("First %d characters are equal\n", n);
    } else if (result < 0) {
        printf("'%s' < '%s' (first %d chars)\n", args[0], args[1], n);
    } else {
        printf("'%s' > '%s' (first %d chars)\n", args[0], args[1], n);
    }
}

void cmd_startswith(char args[][MAX_COMMAND], int arg_count) {
    if (arg_count < 2) {
        printf("Usage: startswith <string> <prefix>\n");
        return;
    }
    
    size_t prefix_len = strlen(args[1]);
    
    if (strncmp(args[0], args[1], prefix_len) == 0) {
        printf("'%s' starts with '%s'\n", args[0], args[1]);
    } else {
        printf("'%s' does not start with '%s'\n", args[0], args[1]);
    }
}

void cmd_match(char args[][MAX_COMMAND], int arg_count) {
    if (arg_count < 2) {
        printf("Usage: match <pattern> <string1> [string2] ...\n");
        return;
    }
    
    printf("Matching pattern '%s':\n", args[0]);
    int matches = 0;
    
    for (int i = 1; i < arg_count; i++) {
        if (strcmp(args[0], args[i]) == 0) {
            printf("  '%s' - EXACT MATCH\n", args[i]);
            matches++;
        } else if (strstr(args[i], args[0]) != NULL) {
            printf("  '%s' - contains pattern\n", args[i]);
            matches++;
        } else {
            printf("  '%s' - no match\n", args[i]);
        }
    }
    
    printf("Total matches: %d\n", matches);
}

// System commands
void cmd_help(void) {
    printf("\n=== Command Interpreter Help ===\n");
    printf("User Management:\n");
    printf("  adduser <user> <pass> [level] - Add new user\n");
    printf("  login <user> <pass>            - Login as user\n");
    printf("  logout                         - Logout current user\n");
    printf("  whoami                         - Show current user\n");
    printf("  listusers                      - List all users\n");
    printf("\nFile Management:\n");
    printf("  createfile <name> [content]    - Create file\n");
    printf("  readfile <name>                - Read file\n");
    printf("  writefile <name> <content>     - Write to file\n");
    printf("  deletefile <name>              - Delete file\n");
    printf("  listfiles                      - List all files\n");
    printf("\nVariable Management:\n");
    printf("  set <name> <value>             - Set variable\n");
    printf("  get <name>                     - Get variable\n");
    printf("  unset <name>                   - Unset variable\n");
    printf("  listvars                       - List all variables\n");
    printf("\nString Operations:\n");
    printf("  compare <str1> <str2>          - Compare strings\n");
    printf("  compareN <str1> <str2> <n>     - Compare first N chars\n");
    printf("  startswith <str> <prefix>      - Check if starts with\n");
    printf("  match <pattern> <str> ...      - Match pattern\n");
    printf("\nSystem:\n");
    printf("  debug [on|off]                 - Toggle debug mode\n");
    printf("  verbose [on|off]               - Toggle verbose mode\n");
    printf("  status                         - Show system status\n");
    printf("  time                           - Show current time\n");
    printf("  help                           - Show this help\n");
    printf("  exit                           - Exit program\n");
}

void cmd_debug(char args[][MAX_COMMAND], int arg_count) {
    if (arg_count < 1) {
        printf("Debug mode: %s\n", debug_mode ? "ON" : "OFF");
        return;
    }
    
    if (strcmp(args[0], "on") == 0) {
        debug_mode = 1;
        printf("Debug mode enabled\n");
    } else if (strcmp(args[0], "off") == 0) {
        debug_mode = 0;
        printf("Debug mode disabled\n");
    } else {
        printf("Usage: debug [on|off]\n");
    }
}

void cmd_verbose(char args[][MAX_COMMAND], int arg_count) {
    if (arg_count < 1) {
        printf("Verbose mode: %s\n", verbose_mode ? "ON" : "OFF");
        return;
    }
    
    if (strcmp(args[0], "on") == 0) {
        verbose_mode = 1;
        printf("Verbose mode enabled\n");
    } else if (strcmp(args[0], "off") == 0) {
        verbose_mode = 0;
        printf("Verbose mode disabled\n");
    } else {
        printf("Usage: verbose [on|off]\n");
    }
}

void cmd_status(void) {
    printf("\n=== System Status ===\n");
    printf("Users: %d/%d\n", user_count, MAX_USERS);
    printf("Files: %d/%d\n", file_count, MAX_FILES);
    printf("Variables: %d/%d\n", variable_count, MAX_VARIABLES);
    printf("Current user: %s\n", 
           (current_user && current_user->logged_in) ? current_user->name : "none");
    printf("Debug mode: %s\n", debug_mode ? "ON" : "OFF");
    printf("Verbose mode: %s\n", verbose_mode ? "ON" : "OFF");
}

void cmd_time(void) {
    time_t now = time(NULL);
    printf("Current time: %s", ctime(&now));
}

// Main command processor using extensive strcmp/strncmp
void process_command(const char *input) {
    char command[MAX_COMMAND];
    char args[MAX_ARGS][MAX_COMMAND];
    int arg_count = 0;
    
    parse_command(input, command, args, &arg_count);
    
    if (strlen(command) == 0) {
        return;
    }
    
    if (debug_mode) {
        printf("[DEBUG] Command: '%s', Args: %d\n", command, arg_count);
    }
    
    // Command routing using strcmp and strncmp
    // User commands
    if (strcmp(command, "adduser") == 0) {
        cmd_adduser(args, arg_count);
    } else if (strcmp(command, "login") == 0) {
        cmd_login(args, arg_count);
    } else if (strcmp(command, "logout") == 0) {
        cmd_logout();
    } else if (strcmp(command, "whoami") == 0) {
        cmd_whoami();
    } else if (strcmp(command, "listusers") == 0 || strcmp(command, "users") == 0) {
        cmd_listusers();
    }
    // File commands
    else if (strcmp(command, "createfile") == 0 || strcmp(command, "touch") == 0) {
        cmd_createfile(args, arg_count);
    } else if (strcmp(command, "readfile") == 0 || strcmp(command, "cat") == 0) {
        cmd_readfile(args, arg_count);
    } else if (strcmp(command, "writefile") == 0 || strcmp(command, "write") == 0) {
        cmd_writefile(args, arg_count);
    } else if (strcmp(command, "deletefile") == 0 || strcmp(command, "rm") == 0) {
        cmd_deletefile(args, arg_count);
    } else if (strcmp(command, "listfiles") == 0 || strcmp(command, "ls") == 0) {
        cmd_listfiles();
    }
    // Variable commands
    else if (strcmp(command, "set") == 0) {
        cmd_set(args, arg_count);
    } else if (strcmp(command, "get") == 0) {
        cmd_get(args, arg_count);
    } else if (strcmp(command, "unset") == 0) {
        cmd_unset(args, arg_count);
    } else if (strcmp(command, "listvars") == 0 || strcmp(command, "vars") == 0) {
        cmd_listvars();
    }
    // String comparison commands
    else if (strcmp(command, "compare") == 0 || strcmp(command, "cmp") == 0) {
        cmd_compare(args, arg_count);
    } else if (strcmp(command, "compareN") == 0 || strcmp(command, "cmpn") == 0) {
        cmd_compareN(args, arg_count);
    } else if (strcmp(command, "startswith") == 0) {
        cmd_startswith(args, arg_count);
    } else if (strcmp(command, "match") == 0) {
        cmd_match(args, arg_count);
    }
    // System commands
    else if (strcmp(command, "debug") == 0) {
        cmd_debug(args, arg_count);
    } else if (strcmp(command, "verbose") == 0) {
        cmd_verbose(args, arg_count);
    } else if (strcmp(command, "status") == 0) {
        cmd_status();
    } else if (strcmp(command, "time") == 0) {
        cmd_time();
    } else if (strcmp(command, "help") == 0 || strcmp(command, "?") == 0) {
        cmd_help();
    } else if (strcmp(command, "exit") == 0 || strcmp(command, "quit") == 0) {
        printf("Goodbye!\n");
        exit(0);
    }
    // Check for partial matches using strncmp
    else if (strncmp(command, "add", 3) == 0) {
        printf("Did you mean 'adduser'?\n");
    } else if (strncmp(command, "log", 3) == 0) {
        printf("Did you mean 'login' or 'logout'?\n");
    } else if (strncmp(command, "list", 4) == 0) {
        printf("Did you mean 'listusers', 'listfiles', or 'listvars'?\n");
    } else if (strncmp(command, "create", 6) == 0) {
        printf("Did you mean 'createfile'?\n");
    } else if (strncmp(command, "read", 4) == 0) {
        printf("Did you mean 'readfile'?\n");
    } else if (strncmp(command, "write", 5) == 0) {
        printf("Did you mean 'writefile'?\n");
    } else if (strncmp(command, "delete", 6) == 0) {
        printf("Did you mean 'deletefile'?\n");
    } else {
        printf("Unknown command: '%s'. Type 'help' for available commands.\n", command);
    }
}

int main(void) {
    printf("|----------------------------------------|\n");
    printf("|   COMMAND INTERPRETER                  |\n");
    printf("|   strcmp/strncmp demonstration         |\n");
    printf("|----------------------------------------|\n");
    printf("Type 'help' for available commands\n\n");
    
    char input[MAX_INPUT];
    
    while (1) {
        printf("> ");
        
        if (fgets(input, MAX_INPUT, stdin) == NULL) {
            break;
        }
        
        // Remove newline
        input[strcspn(input, "\n")] = 0;
        
        if (verbose_mode) {
            printf("[VERBOSE] Processing: '%s'\n", input);
        }
        
        process_command(input);
    }
    
    return 0;
}
