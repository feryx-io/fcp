from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.device_binding_response import DeviceBindingResponse





T = TypeVar("T", bound="CreateLineBindingRequest")



@_attrs_define
class CreateLineBindingRequest:
    """ Create line binding request.

        Attributes:
            phone_number_id (str):
            binding_type (str):
            owner_user_id (None | str | Unset):
            tenant_id (str | Unset): A TenantId: `tenant-<uuid>`, or the bare UUID
            devices (list[DeviceBindingResponse] | None | Unset):
            ring_strategy (None | str | Unset):
            flow_id (None | str | Unset):
            schedule_id (None | str | Unset): A schedule of the tenant; empty clears it.
            closed_target (None | str | Unset): A call target (`voicemail`, `number:+…`, `ext:…`, `flow:…`,
                `queue:…`, a SIP URI); empty clears it.
            holiday_target (None | str | Unset):
            shared (bool | None | Unset): A shared line: its devices share its appearances (§3.9).
            max_appearances (int | None | Unset): How many calls a shared line carries at once (1 to 24).
            barge_in (bool | None | Unset): A phone on the shared line may barge into an active appearance.
            hold_music (None | str | Unset): The number's hold music: a prompt of the tenant, `none` for
                silence, or empty for the tenant's.
     """

    phone_number_id: str
    binding_type: str
    owner_user_id: None | str | Unset = UNSET
    tenant_id: str | Unset = UNSET
    devices: list[DeviceBindingResponse] | None | Unset = UNSET
    ring_strategy: None | str | Unset = UNSET
    flow_id: None | str | Unset = UNSET
    schedule_id: None | str | Unset = UNSET
    closed_target: None | str | Unset = UNSET
    holiday_target: None | str | Unset = UNSET
    shared: bool | None | Unset = UNSET
    max_appearances: int | None | Unset = UNSET
    barge_in: bool | None | Unset = UNSET
    hold_music: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.device_binding_response import DeviceBindingResponse # noqa: PLC0415
        phone_number_id = self.phone_number_id

        binding_type = self.binding_type

        owner_user_id: None | str | Unset
        if isinstance(self.owner_user_id, Unset):
            owner_user_id = UNSET
        else:
            owner_user_id = self.owner_user_id

        tenant_id = self.tenant_id

        devices: list[dict[str, Any]] | None | Unset
        if isinstance(self.devices, Unset):
            devices = UNSET
        elif isinstance(self.devices, list):
            devices = []
            for devices_type_0_item_data in self.devices:
                devices_type_0_item = devices_type_0_item_data.to_dict()
                devices.append(devices_type_0_item)


        else:
            devices = self.devices

        ring_strategy: None | str | Unset
        if isinstance(self.ring_strategy, Unset):
            ring_strategy = UNSET
        else:
            ring_strategy = self.ring_strategy

        flow_id: None | str | Unset
        if isinstance(self.flow_id, Unset):
            flow_id = UNSET
        else:
            flow_id = self.flow_id

        schedule_id: None | str | Unset
        if isinstance(self.schedule_id, Unset):
            schedule_id = UNSET
        else:
            schedule_id = self.schedule_id

        closed_target: None | str | Unset
        if isinstance(self.closed_target, Unset):
            closed_target = UNSET
        else:
            closed_target = self.closed_target

        holiday_target: None | str | Unset
        if isinstance(self.holiday_target, Unset):
            holiday_target = UNSET
        else:
            holiday_target = self.holiday_target

        shared: bool | None | Unset
        if isinstance(self.shared, Unset):
            shared = UNSET
        else:
            shared = self.shared

        max_appearances: int | None | Unset
        if isinstance(self.max_appearances, Unset):
            max_appearances = UNSET
        else:
            max_appearances = self.max_appearances

        barge_in: bool | None | Unset
        if isinstance(self.barge_in, Unset):
            barge_in = UNSET
        else:
            barge_in = self.barge_in

        hold_music: None | str | Unset
        if isinstance(self.hold_music, Unset):
            hold_music = UNSET
        else:
            hold_music = self.hold_music


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "phone_number_id": phone_number_id,
            "binding_type": binding_type,
        })
        if owner_user_id is not UNSET:
            field_dict["owner_user_id"] = owner_user_id
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if devices is not UNSET:
            field_dict["devices"] = devices
        if ring_strategy is not UNSET:
            field_dict["ring_strategy"] = ring_strategy
        if flow_id is not UNSET:
            field_dict["flow_id"] = flow_id
        if schedule_id is not UNSET:
            field_dict["schedule_id"] = schedule_id
        if closed_target is not UNSET:
            field_dict["closed_target"] = closed_target
        if holiday_target is not UNSET:
            field_dict["holiday_target"] = holiday_target
        if shared is not UNSET:
            field_dict["shared"] = shared
        if max_appearances is not UNSET:
            field_dict["max_appearances"] = max_appearances
        if barge_in is not UNSET:
            field_dict["barge_in"] = barge_in
        if hold_music is not UNSET:
            field_dict["hold_music"] = hold_music

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.device_binding_response import DeviceBindingResponse # noqa: PLC0415
        d = dict(src_dict)
        phone_number_id = d.pop("phone_number_id")

        binding_type = d.pop("binding_type")

        def _parse_owner_user_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        owner_user_id = _parse_owner_user_id(d.pop("owner_user_id", UNSET))


        tenant_id = d.pop("tenant_id", UNSET)

        def _parse_devices(data: object) -> list[DeviceBindingResponse] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                devices_type_0 = []
                _devices_type_0 = data
                for devices_type_0_item_data in (_devices_type_0):
                    devices_type_0_item = DeviceBindingResponse.from_dict(devices_type_0_item_data)



                    devices_type_0.append(devices_type_0_item)

                return devices_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[DeviceBindingResponse] | None | Unset, data)

        devices = _parse_devices(d.pop("devices", UNSET))


        def _parse_ring_strategy(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        ring_strategy = _parse_ring_strategy(d.pop("ring_strategy", UNSET))


        def _parse_flow_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        flow_id = _parse_flow_id(d.pop("flow_id", UNSET))


        def _parse_schedule_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        schedule_id = _parse_schedule_id(d.pop("schedule_id", UNSET))


        def _parse_closed_target(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        closed_target = _parse_closed_target(d.pop("closed_target", UNSET))


        def _parse_holiday_target(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        holiday_target = _parse_holiday_target(d.pop("holiday_target", UNSET))


        def _parse_shared(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        shared = _parse_shared(d.pop("shared", UNSET))


        def _parse_max_appearances(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_appearances = _parse_max_appearances(d.pop("max_appearances", UNSET))


        def _parse_barge_in(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        barge_in = _parse_barge_in(d.pop("barge_in", UNSET))


        def _parse_hold_music(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        hold_music = _parse_hold_music(d.pop("hold_music", UNSET))


        create_line_binding_request = cls(
            phone_number_id=phone_number_id,
            binding_type=binding_type,
            owner_user_id=owner_user_id,
            tenant_id=tenant_id,
            devices=devices,
            ring_strategy=ring_strategy,
            flow_id=flow_id,
            schedule_id=schedule_id,
            closed_target=closed_target,
            holiday_target=holiday_target,
            shared=shared,
            max_appearances=max_appearances,
            barge_in=barge_in,
            hold_music=hold_music,
        )


        create_line_binding_request.additional_properties = d
        return create_line_binding_request

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
