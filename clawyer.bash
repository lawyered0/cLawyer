_clawyer() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="clawyer"
                ;;
            clawyer,claude-bridge)
                cmd="clawyer__claude__bridge"
                ;;
            clawyer,completion)
                cmd="clawyer__completion"
                ;;
            clawyer,config)
                cmd="clawyer__config"
                ;;
            clawyer,doctor)
                cmd="clawyer__doctor"
                ;;
            clawyer,help)
                cmd="clawyer__help"
                ;;
            clawyer,mcp)
                cmd="clawyer__mcp"
                ;;
            clawyer,memory)
                cmd="clawyer__memory"
                ;;
            clawyer,onboard)
                cmd="clawyer__onboard"
                ;;
            clawyer,pairing)
                cmd="clawyer__pairing"
                ;;
            clawyer,registry)
                cmd="clawyer__registry"
                ;;
            clawyer,run)
                cmd="clawyer__run"
                ;;
            clawyer,service)
                cmd="clawyer__service"
                ;;
            clawyer,status)
                cmd="clawyer__status"
                ;;
            clawyer,tool)
                cmd="clawyer__tool"
                ;;
            clawyer,worker)
                cmd="clawyer__worker"
                ;;
            clawyer__config,get)
                cmd="clawyer__config__get"
                ;;
            clawyer__config,help)
                cmd="clawyer__config__help"
                ;;
            clawyer__config,init)
                cmd="clawyer__config__init"
                ;;
            clawyer__config,list)
                cmd="clawyer__config__list"
                ;;
            clawyer__config,path)
                cmd="clawyer__config__path"
                ;;
            clawyer__config,reset)
                cmd="clawyer__config__reset"
                ;;
            clawyer__config,set)
                cmd="clawyer__config__set"
                ;;
            clawyer__config__help,get)
                cmd="clawyer__config__help__get"
                ;;
            clawyer__config__help,help)
                cmd="clawyer__config__help__help"
                ;;
            clawyer__config__help,init)
                cmd="clawyer__config__help__init"
                ;;
            clawyer__config__help,list)
                cmd="clawyer__config__help__list"
                ;;
            clawyer__config__help,path)
                cmd="clawyer__config__help__path"
                ;;
            clawyer__config__help,reset)
                cmd="clawyer__config__help__reset"
                ;;
            clawyer__config__help,set)
                cmd="clawyer__config__help__set"
                ;;
            clawyer__help,claude-bridge)
                cmd="clawyer__help__claude__bridge"
                ;;
            clawyer__help,completion)
                cmd="clawyer__help__completion"
                ;;
            clawyer__help,config)
                cmd="clawyer__help__config"
                ;;
            clawyer__help,doctor)
                cmd="clawyer__help__doctor"
                ;;
            clawyer__help,help)
                cmd="clawyer__help__help"
                ;;
            clawyer__help,mcp)
                cmd="clawyer__help__mcp"
                ;;
            clawyer__help,memory)
                cmd="clawyer__help__memory"
                ;;
            clawyer__help,onboard)
                cmd="clawyer__help__onboard"
                ;;
            clawyer__help,pairing)
                cmd="clawyer__help__pairing"
                ;;
            clawyer__help,registry)
                cmd="clawyer__help__registry"
                ;;
            clawyer__help,run)
                cmd="clawyer__help__run"
                ;;
            clawyer__help,service)
                cmd="clawyer__help__service"
                ;;
            clawyer__help,status)
                cmd="clawyer__help__status"
                ;;
            clawyer__help,tool)
                cmd="clawyer__help__tool"
                ;;
            clawyer__help,worker)
                cmd="clawyer__help__worker"
                ;;
            clawyer__help__config,get)
                cmd="clawyer__help__config__get"
                ;;
            clawyer__help__config,init)
                cmd="clawyer__help__config__init"
                ;;
            clawyer__help__config,list)
                cmd="clawyer__help__config__list"
                ;;
            clawyer__help__config,path)
                cmd="clawyer__help__config__path"
                ;;
            clawyer__help__config,reset)
                cmd="clawyer__help__config__reset"
                ;;
            clawyer__help__config,set)
                cmd="clawyer__help__config__set"
                ;;
            clawyer__help__mcp,add)
                cmd="clawyer__help__mcp__add"
                ;;
            clawyer__help__mcp,auth)
                cmd="clawyer__help__mcp__auth"
                ;;
            clawyer__help__mcp,list)
                cmd="clawyer__help__mcp__list"
                ;;
            clawyer__help__mcp,remove)
                cmd="clawyer__help__mcp__remove"
                ;;
            clawyer__help__mcp,test)
                cmd="clawyer__help__mcp__test"
                ;;
            clawyer__help__mcp,toggle)
                cmd="clawyer__help__mcp__toggle"
                ;;
            clawyer__help__memory,read)
                cmd="clawyer__help__memory__read"
                ;;
            clawyer__help__memory,search)
                cmd="clawyer__help__memory__search"
                ;;
            clawyer__help__memory,status)
                cmd="clawyer__help__memory__status"
                ;;
            clawyer__help__memory,tree)
                cmd="clawyer__help__memory__tree"
                ;;
            clawyer__help__memory,write)
                cmd="clawyer__help__memory__write"
                ;;
            clawyer__help__pairing,approve)
                cmd="clawyer__help__pairing__approve"
                ;;
            clawyer__help__pairing,list)
                cmd="clawyer__help__pairing__list"
                ;;
            clawyer__help__registry,info)
                cmd="clawyer__help__registry__info"
                ;;
            clawyer__help__registry,install)
                cmd="clawyer__help__registry__install"
                ;;
            clawyer__help__registry,install-defaults)
                cmd="clawyer__help__registry__install__defaults"
                ;;
            clawyer__help__registry,list)
                cmd="clawyer__help__registry__list"
                ;;
            clawyer__help__service,install)
                cmd="clawyer__help__service__install"
                ;;
            clawyer__help__service,start)
                cmd="clawyer__help__service__start"
                ;;
            clawyer__help__service,status)
                cmd="clawyer__help__service__status"
                ;;
            clawyer__help__service,stop)
                cmd="clawyer__help__service__stop"
                ;;
            clawyer__help__service,uninstall)
                cmd="clawyer__help__service__uninstall"
                ;;
            clawyer__help__tool,auth)
                cmd="clawyer__help__tool__auth"
                ;;
            clawyer__help__tool,info)
                cmd="clawyer__help__tool__info"
                ;;
            clawyer__help__tool,install)
                cmd="clawyer__help__tool__install"
                ;;
            clawyer__help__tool,list)
                cmd="clawyer__help__tool__list"
                ;;
            clawyer__help__tool,remove)
                cmd="clawyer__help__tool__remove"
                ;;
            clawyer__mcp,add)
                cmd="clawyer__mcp__add"
                ;;
            clawyer__mcp,auth)
                cmd="clawyer__mcp__auth"
                ;;
            clawyer__mcp,help)
                cmd="clawyer__mcp__help"
                ;;
            clawyer__mcp,list)
                cmd="clawyer__mcp__list"
                ;;
            clawyer__mcp,remove)
                cmd="clawyer__mcp__remove"
                ;;
            clawyer__mcp,test)
                cmd="clawyer__mcp__test"
                ;;
            clawyer__mcp,toggle)
                cmd="clawyer__mcp__toggle"
                ;;
            clawyer__mcp__help,add)
                cmd="clawyer__mcp__help__add"
                ;;
            clawyer__mcp__help,auth)
                cmd="clawyer__mcp__help__auth"
                ;;
            clawyer__mcp__help,help)
                cmd="clawyer__mcp__help__help"
                ;;
            clawyer__mcp__help,list)
                cmd="clawyer__mcp__help__list"
                ;;
            clawyer__mcp__help,remove)
                cmd="clawyer__mcp__help__remove"
                ;;
            clawyer__mcp__help,test)
                cmd="clawyer__mcp__help__test"
                ;;
            clawyer__mcp__help,toggle)
                cmd="clawyer__mcp__help__toggle"
                ;;
            clawyer__memory,help)
                cmd="clawyer__memory__help"
                ;;
            clawyer__memory,read)
                cmd="clawyer__memory__read"
                ;;
            clawyer__memory,search)
                cmd="clawyer__memory__search"
                ;;
            clawyer__memory,status)
                cmd="clawyer__memory__status"
                ;;
            clawyer__memory,tree)
                cmd="clawyer__memory__tree"
                ;;
            clawyer__memory,write)
                cmd="clawyer__memory__write"
                ;;
            clawyer__memory__help,help)
                cmd="clawyer__memory__help__help"
                ;;
            clawyer__memory__help,read)
                cmd="clawyer__memory__help__read"
                ;;
            clawyer__memory__help,search)
                cmd="clawyer__memory__help__search"
                ;;
            clawyer__memory__help,status)
                cmd="clawyer__memory__help__status"
                ;;
            clawyer__memory__help,tree)
                cmd="clawyer__memory__help__tree"
                ;;
            clawyer__memory__help,write)
                cmd="clawyer__memory__help__write"
                ;;
            clawyer__pairing,approve)
                cmd="clawyer__pairing__approve"
                ;;
            clawyer__pairing,help)
                cmd="clawyer__pairing__help"
                ;;
            clawyer__pairing,list)
                cmd="clawyer__pairing__list"
                ;;
            clawyer__pairing__help,approve)
                cmd="clawyer__pairing__help__approve"
                ;;
            clawyer__pairing__help,help)
                cmd="clawyer__pairing__help__help"
                ;;
            clawyer__pairing__help,list)
                cmd="clawyer__pairing__help__list"
                ;;
            clawyer__registry,help)
                cmd="clawyer__registry__help"
                ;;
            clawyer__registry,info)
                cmd="clawyer__registry__info"
                ;;
            clawyer__registry,install)
                cmd="clawyer__registry__install"
                ;;
            clawyer__registry,install-defaults)
                cmd="clawyer__registry__install__defaults"
                ;;
            clawyer__registry,list)
                cmd="clawyer__registry__list"
                ;;
            clawyer__registry__help,help)
                cmd="clawyer__registry__help__help"
                ;;
            clawyer__registry__help,info)
                cmd="clawyer__registry__help__info"
                ;;
            clawyer__registry__help,install)
                cmd="clawyer__registry__help__install"
                ;;
            clawyer__registry__help,install-defaults)
                cmd="clawyer__registry__help__install__defaults"
                ;;
            clawyer__registry__help,list)
                cmd="clawyer__registry__help__list"
                ;;
            clawyer__service,help)
                cmd="clawyer__service__help"
                ;;
            clawyer__service,install)
                cmd="clawyer__service__install"
                ;;
            clawyer__service,start)
                cmd="clawyer__service__start"
                ;;
            clawyer__service,status)
                cmd="clawyer__service__status"
                ;;
            clawyer__service,stop)
                cmd="clawyer__service__stop"
                ;;
            clawyer__service,uninstall)
                cmd="clawyer__service__uninstall"
                ;;
            clawyer__service__help,help)
                cmd="clawyer__service__help__help"
                ;;
            clawyer__service__help,install)
                cmd="clawyer__service__help__install"
                ;;
            clawyer__service__help,start)
                cmd="clawyer__service__help__start"
                ;;
            clawyer__service__help,status)
                cmd="clawyer__service__help__status"
                ;;
            clawyer__service__help,stop)
                cmd="clawyer__service__help__stop"
                ;;
            clawyer__service__help,uninstall)
                cmd="clawyer__service__help__uninstall"
                ;;
            clawyer__tool,auth)
                cmd="clawyer__tool__auth"
                ;;
            clawyer__tool,help)
                cmd="clawyer__tool__help"
                ;;
            clawyer__tool,info)
                cmd="clawyer__tool__info"
                ;;
            clawyer__tool,install)
                cmd="clawyer__tool__install"
                ;;
            clawyer__tool,list)
                cmd="clawyer__tool__list"
                ;;
            clawyer__tool,remove)
                cmd="clawyer__tool__remove"
                ;;
            clawyer__tool__help,auth)
                cmd="clawyer__tool__help__auth"
                ;;
            clawyer__tool__help,help)
                cmd="clawyer__tool__help__help"
                ;;
            clawyer__tool__help,info)
                cmd="clawyer__tool__help__info"
                ;;
            clawyer__tool__help,install)
                cmd="clawyer__tool__help__install"
                ;;
            clawyer__tool__help,list)
                cmd="clawyer__tool__help__list"
                ;;
            clawyer__tool__help,remove)
                cmd="clawyer__tool__help__remove"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        clawyer)
            opts="-m -c -h -V --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help --version run onboard config tool registry mcp memory pairing service doctor status completion worker claude-bridge help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__claude__bridge)
            opts="-m -c -h --job-id --orchestrator-url --max-turns --model --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --job-id)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --orchestrator-url)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --max-turns)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --model)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__completion)
            opts="-m -c -h --shell --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --shell)
                    COMPREPLY=($(compgen -W "bash elvish fish powershell zsh" -- "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help init list get set reset path help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__get)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <PATH>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__help)
            opts="init list get set reset path help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__help__get)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__help__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__help__init)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__help__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__help__path)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__help__reset)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__help__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__init)
            opts="-o -m -c -h --output --force --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --output)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -o)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__list)
            opts="-f -m -c -h --filter --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --filter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -f)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__path)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__reset)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <PATH>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__config__set)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <PATH> <VALUE>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__doctor)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help)
            opts="run onboard config tool registry mcp memory pairing service doctor status completion worker claude-bridge help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__claude__bridge)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__completion)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__config)
            opts="init list get set reset path"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__config__get)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__config__init)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__config__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__config__path)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__config__reset)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__config__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__doctor)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__mcp)
            opts="add remove list auth test toggle"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__mcp__add)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__mcp__auth)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__mcp__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__mcp__remove)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__mcp__test)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__mcp__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__memory)
            opts="search read write tree status"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__memory__read)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__memory__search)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__memory__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__memory__tree)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__memory__write)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__onboard)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__pairing)
            opts="list approve"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__pairing__approve)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__pairing__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__registry)
            opts="list info install install-defaults"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__registry__info)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__registry__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__registry__install__defaults)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__registry__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__run)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__service)
            opts="install start stop status uninstall"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__service__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__service__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__service__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__service__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__service__uninstall)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__tool)
            opts="install list remove info auth"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__tool__auth)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__tool__info)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__tool__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__tool__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__tool__remove)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__help__worker)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help add remove list auth test toggle help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__add)
            opts="-m -c -h --client-id --auth-url --token-url --scopes --description --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <NAME> <URL>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --client-id)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --auth-url)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --token-url)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --scopes)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__auth)
            opts="-u -m -c -h --user --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --user)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -u)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__help)
            opts="add remove list auth test toggle help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__help__add)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__help__auth)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__help__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__help__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__help__remove)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__help__test)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__help__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__list)
            opts="-v -m -c -h --verbose --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__remove)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__test)
            opts="-u -m -c -h --user --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --user)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -u)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__mcp__toggle)
            opts="-m -c -h --enable --disable --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help search read write tree status help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory__help)
            opts="search read write tree status help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory__help__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory__help__read)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory__help__search)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory__help__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory__help__tree)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory__help__write)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory__read)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <PATH>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory__search)
            opts="-l -m -c -h --limit --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <QUERY>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -l)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory__status)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory__tree)
            opts="-d -m -c -h --depth --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help [PATH]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --depth)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -d)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__memory__write)
            opts="-a -m -c -h --append --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <PATH> [CONTENT]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__onboard)
            opts="-m -c -h --skip-auth --channels-only --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__pairing)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help list approve help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__pairing__approve)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <CHANNEL> <CODE>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__pairing__help)
            opts="list approve help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__pairing__help__approve)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__pairing__help__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__pairing__help__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__pairing__list)
            opts="-m -c -h --json --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <CHANNEL>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__registry)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help list info install install-defaults help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__registry__help)
            opts="list info install install-defaults help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__registry__help__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__registry__help__info)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__registry__help__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__registry__help__install__defaults)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__registry__help__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__registry__info)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__registry__install)
            opts="-f -m -c -h --force --build --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__registry__install__defaults)
            opts="-f -m -c -h --force --build --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__registry__list)
            opts="-k -t -v -m -c -h --kind --tag --verbose --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --kind)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -k)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --tag)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -t)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__run)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help install start stop status uninstall help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service__help)
            opts="install start stop status uninstall help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service__help__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service__help__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service__help__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service__help__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service__help__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service__help__uninstall)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service__install)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service__start)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service__status)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service__stop)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__service__uninstall)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__status)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool)
            opts="-m -c -h --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help install list remove info auth help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool__auth)
            opts="-d -u -m -c -h --dir --user --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --dir)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -d)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --user)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -u)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool__help)
            opts="install list remove info auth help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool__help__auth)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool__help__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool__help__info)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool__help__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool__help__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool__help__remove)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool__info)
            opts="-d -m -c -h --dir --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <NAME_OR_PATH>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --dir)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -d)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool__install)
            opts="-n -t -f -m -c -h --name --capabilities --target --release --skip-build --force --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <PATH>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --name)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -n)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --capabilities)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -t)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool__list)
            opts="-d -v -m -c -h --dir --verbose --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --dir)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -d)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__tool__remove)
            opts="-d -m -c -h --dir --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --dir)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -d)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        clawyer__worker)
            opts="-m -c -h --job-id --orchestrator-url --max-iterations --cli-only --no-db --message --config --no-onboard --matter --jurisdiction --legal-profile --allow-domain --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --job-id)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --orchestrator-url)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --max-iterations)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matter)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --jurisdiction)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --legal-profile)
                    COMPREPLY=($(compgen -W "standard max-lockdown" -- "${cur}"))
                    return 0
                    ;;
                --allow-domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _clawyer -o nosort -o bashdefault -o default clawyer
else
    complete -F _clawyer -o bashdefault -o default clawyer
fi
