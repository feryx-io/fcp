from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.executive_alert import ExecutiveAlert
from ..models.executive_filter import ExecutiveFilter
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="ExecutiveRequest")



@_attrs_define
class ExecutiveRequest:
    """ What may change of a user as an executive.

        Attributes:
            enabled (bool | None | Unset):
            filter_ (ExecutiveFilter | None | Unset):
            alert (ExecutiveAlert | None | Unset):
            assistants (list[str] | None | Unset): The assistants in order, each a username or a user id of the
                tenant; replaces the list, keeping each one's opt-in. An admin's to
                set: refused at `/me/calling`.
     """

    enabled: bool | None | Unset = UNSET
    filter_: ExecutiveFilter | None | Unset = UNSET
    alert: ExecutiveAlert | None | Unset = UNSET
    assistants: list[str] | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        enabled: bool | None | Unset
        if isinstance(self.enabled, Unset):
            enabled = UNSET
        else:
            enabled = self.enabled

        filter_: None | str | Unset
        if isinstance(self.filter_, Unset):
            filter_ = UNSET
        elif isinstance(self.filter_, ExecutiveFilter):
            filter_ = self.filter_.value
        else:
            filter_ = self.filter_

        alert: None | str | Unset
        if isinstance(self.alert, Unset):
            alert = UNSET
        elif isinstance(self.alert, ExecutiveAlert):
            alert = self.alert.value
        else:
            alert = self.alert

        assistants: list[str] | None | Unset
        if isinstance(self.assistants, Unset):
            assistants = UNSET
        elif isinstance(self.assistants, list):
            assistants = self.assistants


        else:
            assistants = self.assistants


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if filter_ is not UNSET:
            field_dict["filter"] = filter_
        if alert is not UNSET:
            field_dict["alert"] = alert
        if assistants is not UNSET:
            field_dict["assistants"] = assistants

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        enabled = _parse_enabled(d.pop("enabled", UNSET))


        def _parse_filter_(data: object) -> ExecutiveFilter | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                filter_type_0 = ExecutiveFilter(data)



                return filter_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(ExecutiveFilter | None | Unset, data)

        filter_ = _parse_filter_(d.pop("filter", UNSET))


        def _parse_alert(data: object) -> ExecutiveAlert | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                alert_type_0 = ExecutiveAlert(data)



                return alert_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(ExecutiveAlert | None | Unset, data)

        alert = _parse_alert(d.pop("alert", UNSET))


        def _parse_assistants(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                assistants_type_0 = cast(list[str], data)

                return assistants_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        assistants = _parse_assistants(d.pop("assistants", UNSET))


        executive_request = cls(
            enabled=enabled,
            filter_=filter_,
            alert=alert,
            assistants=assistants,
        )


        executive_request.additional_properties = d
        return executive_request

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
