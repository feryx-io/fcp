from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.parked_caller import ParkedCaller





T = TypeVar("T", bound="ParkedCallResponse")



@_attrs_define
class ParkedCallResponse:
    """ A call waiting in a slot, on whichever call manager holds it.

        Attributes:
            slot (str):
            lot_id (str): The lot; `default` for the node's own range.
            call_id (str):
            parker_extension (str):
            parked_secs (int): How long it has waited, and how long it may.
            timeout_secs (int):
            node_id (str): The call manager holding it.
            caller (None | ParkedCaller | Unset):
            site_id (None | str | Unset): A site's call (`docs/PARKING.md` phase 4), as its link last told
                the cloud: taken back or hung up at the site, not here.
            site_name (None | str | Unset):
     """

    slot: str
    lot_id: str
    call_id: str
    parker_extension: str
    parked_secs: int
    timeout_secs: int
    node_id: str
    caller: None | ParkedCaller | Unset = UNSET
    site_id: None | str | Unset = UNSET
    site_name: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.parked_caller import ParkedCaller # noqa: PLC0415
        slot = self.slot

        lot_id = self.lot_id

        call_id = self.call_id

        parker_extension = self.parker_extension

        parked_secs = self.parked_secs

        timeout_secs = self.timeout_secs

        node_id = self.node_id

        caller: dict[str, Any] | None | Unset
        if isinstance(self.caller, Unset):
            caller = UNSET
        elif isinstance(self.caller, ParkedCaller):
            caller = self.caller.to_dict()
        else:
            caller = self.caller

        site_id: None | str | Unset
        if isinstance(self.site_id, Unset):
            site_id = UNSET
        else:
            site_id = self.site_id

        site_name: None | str | Unset
        if isinstance(self.site_name, Unset):
            site_name = UNSET
        else:
            site_name = self.site_name


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "slot": slot,
            "lot_id": lot_id,
            "call_id": call_id,
            "parker_extension": parker_extension,
            "parked_secs": parked_secs,
            "timeout_secs": timeout_secs,
            "node_id": node_id,
        })
        if caller is not UNSET:
            field_dict["caller"] = caller
        if site_id is not UNSET:
            field_dict["site_id"] = site_id
        if site_name is not UNSET:
            field_dict["site_name"] = site_name

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.parked_caller import ParkedCaller # noqa: PLC0415
        d = dict(src_dict)
        slot = d.pop("slot")

        lot_id = d.pop("lot_id")

        call_id = d.pop("call_id")

        parker_extension = d.pop("parker_extension")

        parked_secs = d.pop("parked_secs")

        timeout_secs = d.pop("timeout_secs")

        node_id = d.pop("node_id")

        def _parse_caller(data: object) -> None | ParkedCaller | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                caller_type_0 = ParkedCaller.from_dict(data)



                return caller_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | ParkedCaller | Unset, data)

        caller = _parse_caller(d.pop("caller", UNSET))


        def _parse_site_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        site_id = _parse_site_id(d.pop("site_id", UNSET))


        def _parse_site_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        site_name = _parse_site_name(d.pop("site_name", UNSET))


        parked_call_response = cls(
            slot=slot,
            lot_id=lot_id,
            call_id=call_id,
            parker_extension=parker_extension,
            parked_secs=parked_secs,
            timeout_secs=timeout_secs,
            node_id=node_id,
            caller=caller,
            site_id=site_id,
            site_name=site_name,
        )


        parked_call_response.additional_properties = d
        return parked_call_response

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
