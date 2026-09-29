from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="TrunkNodeView")



@_attrs_define
class TrunkNodeView:
    """ What one node sees of one trunk.

        Attributes:
            trunk_id (str):
            name (str):
            enabled (bool):
            health (str): `healthy`, `degraded`, `down` or `unknown` — what this node's own
                probes have found, not what another node found.
            consecutive_failures (int):
            calls_in_flight (int): Calls this node has on the trunk right now. Capacity is per node
                (§13.3), so this is a share of the carrier's channels, not all of
                them.
            routable (bool): Whether this node would route a call to the trunk now: enabled,
                not administratively disabled and not down.
            node_id (None | str | Unset): The node this is the view of. Absent inside a node's own report,
                which names its node once; set where views of one trunk from many
                nodes are gathered, as `/v1/trunks` does.
            tenant_id (None | str | Unset): The platform's own trunks have no tenant.
            hosts_down (list[str] | Unset): Hosts of the trunk this node's last probe found down; a call tries
                them after the ones that answered (`docs/DEPLOYMENT_GAPS.md` §1.8).
            registration (None | str | Unset): For a registering trunk, what this node's binding is doing:
                `registered`, `pending`, `failed` or `unregistered`. Absent on a
                trunk that does not register, and `unregistered` on one another
                node holds.
            account (None | str | Unset): The carrier account bound, when the trunk registers as itself.
     """

    trunk_id: str
    name: str
    enabled: bool
    health: str
    consecutive_failures: int
    calls_in_flight: int
    routable: bool
    node_id: None | str | Unset = UNSET
    tenant_id: None | str | Unset = UNSET
    hosts_down: list[str] | Unset = UNSET
    registration: None | str | Unset = UNSET
    account: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        trunk_id = self.trunk_id

        name = self.name

        enabled = self.enabled

        health = self.health

        consecutive_failures = self.consecutive_failures

        calls_in_flight = self.calls_in_flight

        routable = self.routable

        node_id: None | str | Unset
        if isinstance(self.node_id, Unset):
            node_id = UNSET
        else:
            node_id = self.node_id

        tenant_id: None | str | Unset
        if isinstance(self.tenant_id, Unset):
            tenant_id = UNSET
        else:
            tenant_id = self.tenant_id

        hosts_down: list[str] | Unset = UNSET
        if not isinstance(self.hosts_down, Unset):
            hosts_down = self.hosts_down



        registration: None | str | Unset
        if isinstance(self.registration, Unset):
            registration = UNSET
        else:
            registration = self.registration

        account: None | str | Unset
        if isinstance(self.account, Unset):
            account = UNSET
        else:
            account = self.account


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "trunk_id": trunk_id,
            "name": name,
            "enabled": enabled,
            "health": health,
            "consecutive_failures": consecutive_failures,
            "calls_in_flight": calls_in_flight,
            "routable": routable,
        })
        if node_id is not UNSET:
            field_dict["node_id"] = node_id
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if hosts_down is not UNSET:
            field_dict["hosts_down"] = hosts_down
        if registration is not UNSET:
            field_dict["registration"] = registration
        if account is not UNSET:
            field_dict["account"] = account

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        trunk_id = d.pop("trunk_id")

        name = d.pop("name")

        enabled = d.pop("enabled")

        health = d.pop("health")

        consecutive_failures = d.pop("consecutive_failures")

        calls_in_flight = d.pop("calls_in_flight")

        routable = d.pop("routable")

        def _parse_node_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        node_id = _parse_node_id(d.pop("node_id", UNSET))


        def _parse_tenant_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tenant_id = _parse_tenant_id(d.pop("tenant_id", UNSET))


        hosts_down = cast(list[str], d.pop("hosts_down", UNSET))


        def _parse_registration(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        registration = _parse_registration(d.pop("registration", UNSET))


        def _parse_account(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        account = _parse_account(d.pop("account", UNSET))


        trunk_node_view = cls(
            trunk_id=trunk_id,
            name=name,
            enabled=enabled,
            health=health,
            consecutive_failures=consecutive_failures,
            calls_in_flight=calls_in_flight,
            routable=routable,
            node_id=node_id,
            tenant_id=tenant_id,
            hosts_down=hosts_down,
            registration=registration,
            account=account,
        )


        trunk_node_view.additional_properties = d
        return trunk_node_view

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
