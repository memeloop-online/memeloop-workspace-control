//go:build linux

// mwc-flannel-neighbor clears the previous owner's host ARP entry after a CNI
// bridge assigns an IPv4 address to a new pod. The kernel relearns its MAC.
package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"

	"github.com/containernetworking/cni/pkg/skel"
	"github.com/containernetworking/cni/pkg/types"
	current "github.com/containernetworking/cni/pkg/types/100"
	"github.com/containernetworking/cni/pkg/version"
	"github.com/vishvananda/netlink"
	"github.com/vishvananda/netlink/nl"
	"golang.org/x/sys/unix"
)

type config struct {
	types.NetConf
	Bridge string `json:"bridge"`
}

func main() {
	noop := func(*skel.CmdArgs) error { return nil }
	skel.PluginMainFuncs(skel.CNIFuncs{Add: add, Del: noop, Check: noop},
		version.PluginSupports("1.0.0"), "MWC Flannel bridge IPv4 neighbor cleanup")
}

func add(args *skel.CmdArgs) error {
	var conf config
	if err := json.Unmarshal(args.StdinData, &conf); err != nil {
		return err
	}
	if conf.Bridge == "" || conf.RawPrevResult == nil {
		return errors.New("mwc-flannel-neighbor requires bridge and prevResult")
	}
	if err := version.ParsePrevResult(&conf.NetConf); err != nil {
		return err
	}
	result, err := current.NewResultFromResult(conf.PrevResult)
	if err != nil {
		return err
	}
	if err := clearPreviousNeighbors(conf.Bridge, args, result); err != nil {
		// Keep the working CNI result if optional host cache cleanup fails.
		fmt.Fprintf(os.Stderr, "mwc-flannel-neighbor: %v\n", err)
	}
	return types.PrintResult(result, conf.CNIVersion)
}

func clearPreviousNeighbors(bridge string, args *skel.CmdArgs, result *current.Result) error {
	bridgeReturned, sandboxIndex := false, -1
	var mac net.HardwareAddr
	for i, iface := range result.Interfaces {
		switch {
		case iface.Sandbox == "" && iface.Name == bridge:
			bridgeReturned = true
		case iface.Sandbox == args.Netns && iface.Name == args.IfName:
			sandboxIndex = i
			mac, _ = net.ParseMAC(iface.Mac)
		}
	}
	if !bridgeReturned || sandboxIndex < 0 || len(mac) != 6 {
		return nil
	}
	link, err := netlink.LinkByName(bridge)
	if err != nil {
		return err
	}
	if link.Type() != "bridge" {
		return fmt.Errorf("%s is not a bridge", bridge)
	}
	var cleanupErr error
	for _, ip := range result.IPs {
		addr := ip.Address.IP.To4()
		if ip.Interface != nil && *ip.Interface == sandboxIndex && addr != nil {
			if err := clearNeighbor(link.Attrs().Index, addr, mac); err != nil {
				cleanupErr = errors.Join(cleanupErr, fmt.Errorf("%s on %s: %w", addr, bridge, err))
			}
		}
	}
	return cleanupErr
}

func clearNeighbor(ifindex int, ip net.IP, mac net.HardwareAddr) error {
	// Query this address on this interface; do not dump the host's ARP table.
	req := nl.NewNetlinkRequest(unix.RTM_GETNEIGH, unix.NLM_F_ACK)
	req.AddData(&netlink.Ndmsg{Family: unix.AF_INET, Index: uint32(ifindex)})
	req.AddData(nl.NewRtAttr(netlink.NDA_DST, ip))
	messages, err := req.Execute(unix.NETLINK_ROUTE, unix.RTM_NEWNEIGH)
	if errors.Is(err, unix.ENOENT) {
		return nil
	}
	if err != nil {
		return err
	}
	for _, message := range messages {
		neighbor, err := netlink.NeighDeserialize(message)
		if err != nil {
			return err
		}
		if neighbor.LinkIndex != ifindex || !neighbor.IP.Equal(ip) ||
			neighbor.State&(netlink.NUD_PERMANENT|netlink.NUD_NOARP) != 0 ||
			neighbor.Flags&netlink.NTF_PROXY != 0 || len(neighbor.HardwareAddr) == 0 ||
			bytes.Equal(neighbor.HardwareAddr, mac) {
			continue
		}
		err = netlink.NeighDel(neighbor)
		if err != nil && !errors.Is(err, unix.ENOENT) && !errors.Is(err, unix.ESRCH) {
			return err
		}
	}
	return nil
}
