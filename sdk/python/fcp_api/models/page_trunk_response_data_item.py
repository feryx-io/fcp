from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.trunk_node_view import TrunkNodeView





T = TypeVar("T", bound="PageTrunkResponseDataItem")



@_attrs_define
class PageTrunkResponseDataItem:
    """ Trunk response.

        Attributes:
            id (str):
            tenant_id (str):
            name (str):
            enabled (bool):
            direction (str):
            priority (int):
            weight (int):
            hosts (list[str]):
            inbound_sources (list[str]):
            transport_protocol (str):
            transport_port (int):
            verify_tls (bool):
            has_credentials (bool):
            register (bool):
            register_expiry (int):
            max_concurrent_calls (int):
            max_failures (int):
            recovery_interval_secs (int):
            health_check_enabled (bool):
            emergency_capable (bool):
            health_check_interval_secs (int):
            codec_prefs (list[str]):
            strip_digits (int):
            cps_limit (int):
            srtp_mode (str):
            dtmf_mode (str):
            session_timer_secs (int):
            t38_enabled (bool):
            health_status (str):
            consecutive_failures (int):
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            site_id (None | str | Unset): The site this trunk runs at, in a hybrid deployment
                (`docs/HYBRID_CONTROL_PLANE.md` §3.6); absent for the cloud's own.
            link (None | str | Unset): `cloud` or `site`: which end of a site's link this trunk is, when
                it is one. Made at enrollment, never set through this API.
            auth_username (None | str | Unset):
            auth_realm (None | str | Unset):
            caller_id_override (None | str | Unset):
            from_domain (None | str | Unset):
            prepend_digits (None | str | Unset):
            description (None | str | Unset):
            outbound_proxy (None | str | Unset):
            trunk_group (None | str | Unset):
            header_rules (None | str | Unset):
            uptime_secs (int | None | Unset):
            nodes (list[TrunkNodeView] | Unset): What each node that routes by this trunk sees of it
                (`docs/TENANT_TRUNKS.md` §9): a carrier one node cannot reach may
                be reachable from another, and that disagreement is worth showing.
                Empty where no node publishes a view.
            calls_in_flight (int | Unset): Calls on this trunk across every node that reported one.
            routable_somewhere (bool | Unset): Whether any node would route a call to it now.
     """

    id: str
    tenant_id: str
    name: str
    enabled: bool
    direction: str
    priority: int
    weight: int
    hosts: list[str]
    inbound_sources: list[str]
    transport_protocol: str
    transport_port: int
    verify_tls: bool
    has_credentials: bool
    register: bool
    register_expiry: int
    max_concurrent_calls: int
    max_failures: int
    recovery_interval_secs: int
    health_check_enabled: bool
    emergency_capable: bool
    health_check_interval_secs: int
    codec_prefs: list[str]
    strip_digits: int
    cps_limit: int
    srtp_mode: str
    dtmf_mode: str
    session_timer_secs: int
    t38_enabled: bool
    health_status: str
    consecutive_failures: int
    created_at: datetime.datetime
    updated_at: datetime.datetime
    site_id: None | str | Unset = UNSET
    link: None | str | Unset = UNSET
    auth_username: None | str | Unset = UNSET
    auth_realm: None | str | Unset = UNSET
    caller_id_override: None | str | Unset = UNSET
    from_domain: None | str | Unset = UNSET
    prepend_digits: None | str | Unset = UNSET
    description: None | str | Unset = UNSET
    outbound_proxy: None | str | Unset = UNSET
    trunk_group: None | str | Unset = UNSET
    header_rules: None | str | Unset = UNSET
    uptime_secs: int | None | Unset = UNSET
    nodes: list[TrunkNodeView] | Unset = UNSET
    calls_in_flight: int | Unset = UNSET
    routable_somewhere: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.trunk_node_view import TrunkNodeView # noqa: PLC0415
        id = self.id

        tenant_id = self.tenant_id

        name = self.name

        enabled = self.enabled

        direction = self.direction

        priority = self.priority

        weight = self.weight

        hosts = self.hosts



        inbound_sources = self.inbound_sources



        transport_protocol = self.transport_protocol

        transport_port = self.transport_port

        verify_tls = self.verify_tls

        has_credentials = self.has_credentials

        register = self.register

        register_expiry = self.register_expiry

        max_concurrent_calls = self.max_concurrent_calls

        max_failures = self.max_failures

        recovery_interval_secs = self.recovery_interval_secs

        health_check_enabled = self.health_check_enabled

        emergency_capable = self.emergency_capable

        health_check_interval_secs = self.health_check_interval_secs

        codec_prefs = self.codec_prefs



        strip_digits = self.strip_digits

        cps_limit = self.cps_limit

        srtp_mode = self.srtp_mode

        dtmf_mode = self.dtmf_mode

        session_timer_secs = self.session_timer_secs

        t38_enabled = self.t38_enabled

        health_status = self.health_status

        consecutive_failures = self.consecutive_failures

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        site_id: None | str | Unset
        if isinstance(self.site_id, Unset):
            site_id = UNSET
        else:
            site_id = self.site_id

        link: None | str | Unset
        if isinstance(self.link, Unset):
            link = UNSET
        else:
            link = self.link

        auth_username: None | str | Unset
        if isinstance(self.auth_username, Unset):
            auth_username = UNSET
        else:
            auth_username = self.auth_username

        auth_realm: None | str | Unset
        if isinstance(self.auth_realm, Unset):
            auth_realm = UNSET
        else:
            auth_realm = self.auth_realm

        caller_id_override: None | str | Unset
        if isinstance(self.caller_id_override, Unset):
            caller_id_override = UNSET
        else:
            caller_id_override = self.caller_id_override

        from_domain: None | str | Unset
        if isinstance(self.from_domain, Unset):
            from_domain = UNSET
        else:
            from_domain = self.from_domain

        prepend_digits: None | str | Unset
        if isinstance(self.prepend_digits, Unset):
            prepend_digits = UNSET
        else:
            prepend_digits = self.prepend_digits

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        outbound_proxy: None | str | Unset
        if isinstance(self.outbound_proxy, Unset):
            outbound_proxy = UNSET
        else:
            outbound_proxy = self.outbound_proxy

        trunk_group: None | str | Unset
        if isinstance(self.trunk_group, Unset):
            trunk_group = UNSET
        else:
            trunk_group = self.trunk_group

        header_rules: None | str | Unset
        if isinstance(self.header_rules, Unset):
            header_rules = UNSET
        else:
            header_rules = self.header_rules

        uptime_secs: int | None | Unset
        if isinstance(self.uptime_secs, Unset):
            uptime_secs = UNSET
        else:
            uptime_secs = self.uptime_secs

        nodes: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.nodes, Unset):
            nodes = []
            for nodes_item_data in self.nodes:
                nodes_item = nodes_item_data.to_dict()
                nodes.append(nodes_item)



        calls_in_flight = self.calls_in_flight

        routable_somewhere = self.routable_somewhere


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "name": name,
            "enabled": enabled,
            "direction": direction,
            "priority": priority,
            "weight": weight,
            "hosts": hosts,
            "inbound_sources": inbound_sources,
            "transport_protocol": transport_protocol,
            "transport_port": transport_port,
            "verify_tls": verify_tls,
            "has_credentials": has_credentials,
            "register": register,
            "register_expiry": register_expiry,
            "max_concurrent_calls": max_concurrent_calls,
            "max_failures": max_failures,
            "recovery_interval_secs": recovery_interval_secs,
            "health_check_enabled": health_check_enabled,
            "emergency_capable": emergency_capable,
            "health_check_interval_secs": health_check_interval_secs,
            "codec_prefs": codec_prefs,
            "strip_digits": strip_digits,
            "cps_limit": cps_limit,
            "srtp_mode": srtp_mode,
            "dtmf_mode": dtmf_mode,
            "session_timer_secs": session_timer_secs,
            "t38_enabled": t38_enabled,
            "health_status": health_status,
            "consecutive_failures": consecutive_failures,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if site_id is not UNSET:
            field_dict["site_id"] = site_id
        if link is not UNSET:
            field_dict["link"] = link
        if auth_username is not UNSET:
            field_dict["auth_username"] = auth_username
        if auth_realm is not UNSET:
            field_dict["auth_realm"] = auth_realm
        if caller_id_override is not UNSET:
            field_dict["caller_id_override"] = caller_id_override
        if from_domain is not UNSET:
            field_dict["from_domain"] = from_domain
        if prepend_digits is not UNSET:
            field_dict["prepend_digits"] = prepend_digits
        if description is not UNSET:
            field_dict["description"] = description
        if outbound_proxy is not UNSET:
            field_dict["outbound_proxy"] = outbound_proxy
        if trunk_group is not UNSET:
            field_dict["trunk_group"] = trunk_group
        if header_rules is not UNSET:
            field_dict["header_rules"] = header_rules
        if uptime_secs is not UNSET:
            field_dict["uptime_secs"] = uptime_secs
        if nodes is not UNSET:
            field_dict["nodes"] = nodes
        if calls_in_flight is not UNSET:
            field_dict["calls_in_flight"] = calls_in_flight
        if routable_somewhere is not UNSET:
            field_dict["routable_somewhere"] = routable_somewhere

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.trunk_node_view import TrunkNodeView # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        name = d.pop("name")

        enabled = d.pop("enabled")

        direction = d.pop("direction")

        priority = d.pop("priority")

        weight = d.pop("weight")

        hosts = cast(list[str], d.pop("hosts"))


        inbound_sources = cast(list[str], d.pop("inbound_sources"))


        transport_protocol = d.pop("transport_protocol")

        transport_port = d.pop("transport_port")

        verify_tls = d.pop("verify_tls")

        has_credentials = d.pop("has_credentials")

        register = d.pop("register")

        register_expiry = d.pop("register_expiry")

        max_concurrent_calls = d.pop("max_concurrent_calls")

        max_failures = d.pop("max_failures")

        recovery_interval_secs = d.pop("recovery_interval_secs")

        health_check_enabled = d.pop("health_check_enabled")

        emergency_capable = d.pop("emergency_capable")

        health_check_interval_secs = d.pop("health_check_interval_secs")

        codec_prefs = cast(list[str], d.pop("codec_prefs"))


        strip_digits = d.pop("strip_digits")

        cps_limit = d.pop("cps_limit")

        srtp_mode = d.pop("srtp_mode")

        dtmf_mode = d.pop("dtmf_mode")

        session_timer_secs = d.pop("session_timer_secs")

        t38_enabled = d.pop("t38_enabled")

        health_status = d.pop("health_status")

        consecutive_failures = d.pop("consecutive_failures")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_site_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        site_id = _parse_site_id(d.pop("site_id", UNSET))


        def _parse_link(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        link = _parse_link(d.pop("link", UNSET))


        def _parse_auth_username(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        auth_username = _parse_auth_username(d.pop("auth_username", UNSET))


        def _parse_auth_realm(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        auth_realm = _parse_auth_realm(d.pop("auth_realm", UNSET))


        def _parse_caller_id_override(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        caller_id_override = _parse_caller_id_override(d.pop("caller_id_override", UNSET))


        def _parse_from_domain(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        from_domain = _parse_from_domain(d.pop("from_domain", UNSET))


        def _parse_prepend_digits(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        prepend_digits = _parse_prepend_digits(d.pop("prepend_digits", UNSET))


        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_outbound_proxy(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        outbound_proxy = _parse_outbound_proxy(d.pop("outbound_proxy", UNSET))


        def _parse_trunk_group(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        trunk_group = _parse_trunk_group(d.pop("trunk_group", UNSET))


        def _parse_header_rules(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        header_rules = _parse_header_rules(d.pop("header_rules", UNSET))


        def _parse_uptime_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        uptime_secs = _parse_uptime_secs(d.pop("uptime_secs", UNSET))


        _nodes = d.pop("nodes", UNSET)
        nodes: list[TrunkNodeView] | Unset = UNSET
        if _nodes is not UNSET:
            nodes = []
            for nodes_item_data in _nodes:
                nodes_item = TrunkNodeView.from_dict(nodes_item_data)



                nodes.append(nodes_item)


        calls_in_flight = d.pop("calls_in_flight", UNSET)

        routable_somewhere = d.pop("routable_somewhere", UNSET)

        page_trunk_response_data_item = cls(
            id=id,
            tenant_id=tenant_id,
            name=name,
            enabled=enabled,
            direction=direction,
            priority=priority,
            weight=weight,
            hosts=hosts,
            inbound_sources=inbound_sources,
            transport_protocol=transport_protocol,
            transport_port=transport_port,
            verify_tls=verify_tls,
            has_credentials=has_credentials,
            register=register,
            register_expiry=register_expiry,
            max_concurrent_calls=max_concurrent_calls,
            max_failures=max_failures,
            recovery_interval_secs=recovery_interval_secs,
            health_check_enabled=health_check_enabled,
            emergency_capable=emergency_capable,
            health_check_interval_secs=health_check_interval_secs,
            codec_prefs=codec_prefs,
            strip_digits=strip_digits,
            cps_limit=cps_limit,
            srtp_mode=srtp_mode,
            dtmf_mode=dtmf_mode,
            session_timer_secs=session_timer_secs,
            t38_enabled=t38_enabled,
            health_status=health_status,
            consecutive_failures=consecutive_failures,
            created_at=created_at,
            updated_at=updated_at,
            site_id=site_id,
            link=link,
            auth_username=auth_username,
            auth_realm=auth_realm,
            caller_id_override=caller_id_override,
            from_domain=from_domain,
            prepend_digits=prepend_digits,
            description=description,
            outbound_proxy=outbound_proxy,
            trunk_group=trunk_group,
            header_rules=header_rules,
            uptime_secs=uptime_secs,
            nodes=nodes,
            calls_in_flight=calls_in_flight,
            routable_somewhere=routable_somewhere,
        )


        page_trunk_response_data_item.additional_properties = d
        return page_trunk_response_data_item

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
